# Pin groups end to end: the number of VISIBLE "Cuter 贴图" windows is the assertion.
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\pin_groups_e2e.ps1
#
# Scenario (2 groups is the default):
#
#   1. put an image on the clipboard, Alt+V          -> 1 visible pin   (group 1)
#   2. Alt+G (next group)                         -> 0 visible pins  (group 2 is empty)
#   3. Alt+V                                        -> 1 visible pin   (now in group 2)
#   4. Alt+G (back to group 1)                    -> 1 visible pin   (group 2 hidden)
#   5. Alt+H (hide)                              -> 0 visible pins
#   6. Alt+H (show)                              -> 1 visible pin
#
# Step 2 is the interesting one: a group switch must HIDE the other group's pins without
# destroying them - which step 4 then proves by bringing the first pin back.
param(
    [string]$Exe = ''
)

$ErrorActionPreference = 'Stop'

function Say($m) { Write-Host $m }

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - build it first" }
$exe = (Resolve-Path $exe).Path
Say "probing $exe"

Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-pingroup-config-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$logDir = Join-Path $env:APPDATA 'Cuter\logs'
$logPath = Join-Path $logDir 'cuter.log'
if (-not (Test-Path $configBackup)) { Copy-Item $configPath $configBackup -Force }

$pinTitle = 'Cuter ' + [char]0x8D34 + [char]0x56FE     # "Cuter 贴图"
$script:blocked = $false

function Blocked-Message([string]$text) {
    $fg = [CuterInputProbe]::ForegroundProcess()
    if ($fg -and $fg -notmatch '^(cuter|powershell|pwsh|cmd|conhost|explorer|WindowsTerminal)$') {
        $script:blocked = $true
        Say "foreground app is '$fg' - [$([CuterInputProbe]::ForegroundTitle())]"
        Say '  (that is not us: a VM / remote-desktop window swallows synthesized keys)'
        return "SKIPPED_KEYBOARD_GRABBED: $text"
    }
    return $text
}

function Stop-AllCuter {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 100
        if (-not (Get-Process cuter -ErrorAction SilentlyContinue)) { return }
    }
    throw 'a cuter process is still running'
}

trap {
    if ($script:blocked -or $_.Exception.Message -like 'SKIPPED_KEYBOARD_GRABBED*') {
        Write-Host "SKIPPED: $($_.Exception.Message -replace '^SKIPPED_KEYBOARD_GRABBED: ', '')"
    } else {
        Write-Host "FAILED: $($_.Exception.Message)"
    }
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
    if ($script:blocked) {
        Write-Host 'RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive'
        exit 2
    }
    exit 1
}

# Deterministic starting point: 2 groups, no leftover pins, tray mode.
New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true

[hotkeys]
capture = "Alt+C"
toggle_pins = "Alt+H"
next_pin_group = "Alt+G"
pin_from_clipboard = "Alt+V"

[pin]
opacity = 0.95
shadow = false
border = true
groups = 2
active_group = 0

[history]
limit = 0

[ocr]
engine = "disabled"
"@

# Clipboard image (the probes run in STA, so Windows.Forms clipboard works).
$bmp = New-Object System.Drawing.Bitmap(240, 160)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.Clear([System.Drawing.Color]::FromArgb(255, 30, 120, 200))
$g.Dispose()
[System.Windows.Forms.Clipboard]::SetImage($bmp)
$bmp.Dispose()
Say 'clipboard: image set'

Stop-AllCuter
$before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
$app = Start-Process $exe -PassThru
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 60000) {
    if ((Test-Path $logPath) -and ((Get-Item $logPath).Length -gt $before)) { break }
    $app.Refresh()
    if ($app.HasExited) { throw "the app exited immediately (exit code $($app.ExitCode))" }
    Start-Sleep -Milliseconds 200
}
Start-Sleep -Seconds 2
[void][CuterInputProbe]::FocusDesktop()
Start-Sleep -Milliseconds 400

# ---------------------------------------------------------------------------
# This probe deliberately uses **no keyboard at all**.
#
# The first version drove Alt+V / Alt+G / Alt+H. That was flaky for two reasons that both
# look like feature bugs: (a) a running VM or the Windows search flyout swallows
# synthesized keys, and (b) the hotkeys themselves may fail to register because another
# program owns the combination (Shift+F3 / Ctrl+F3 are often taken - which is why our
# defaults are Alt+H / Alt+G now).
#
# So the actions go through `cuter --pin` / `--next-pin-group` / `--toggle-pins`, which are
# forwarded to the running instance over the single-instance channel: same code path in the
# app, no keyboard involved.
# ---------------------------------------------------------------------------

# Drive the actions through the CLI, which forwards to the running instance over the
# single-instance channel. **That path does not touch the keyboard at all**, so this probe
# stays deterministic even when a VM / search flyout owns the keyboard (which is exactly
# what made the hotkey version flaky).
function Invoke-Action([string]$arg, [string]$what) {
    [CuterInputProbe]::MoveTo(960, 540) | Out-Null   # keep the cursor away from pins
    & $exe $arg | Out-Null
    Start-Sleep -Milliseconds 1200
    $count = [CuterInputProbe]::CountWindows([uint32]$app.Id, $pinTitle)
    Say ("{0,-42} visible pins = {1}" -f $what, $count)
    return $count
}

$fail = @()
$step1 = Invoke-Action '--pin' 'pin from clipboard (group 1)  # 1'
if ($step1 -ne 1) { $fail += "Alt+V should show exactly 1 pin, got $step1" }

$step2 = Invoke-Action '--next-pin-group' 'switch to group 2            # 2'
if ($step2 -ne 0) { $fail += "switching to the empty group 2 should hide group 1's pin, got $step2 visible" }

$step3 = Invoke-Action '--pin' 'another pin (now in group 2)   # 3'
if ($step3 -ne 1) { $fail += "a pin created in group 2 should be visible, got $step3" }

$step4 = Invoke-Action '--next-pin-group' 'back to group 1             # 4'
if ($step4 -ne 1) { $fail += "switching back should reveal group 1's pin (and hide group 2's), got $step4" }

$step5 = Invoke-Action '--toggle-pins' 'hide this group              # 5'
if ($step5 -ne 0) { $fail += "Shift+F3 should hide the current group, got $step5 visible" }

$step6 = Invoke-Action '--toggle-pins' 'show again                   # 6'
if ($step6 -ne 1) { $fail += "Shift+F3 again should bring the group back, got $step6 visible" }

Stop-AllCuter
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }

if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw 'pin groups e2e failed'
}
Say 'RESULT: PASS - pins live in groups, switching hides/shows without destroying them'
