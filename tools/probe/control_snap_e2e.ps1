# Control-level snapping (UI Automation) end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\control_snap_e2e.ps1
#
# Target: a window (tools/probe/ControlWindow.cs) with a native BUTTON whose rect is known
# exactly. The app snapshots the desktop, so "what got selected" is proved by the SIZE of
# the PNG the app saves when the selection is confirmed - the strongest assertion available
# without poking at the app's internals.
#
#   A. click ON the button        -> the saved PNG must be exactly the button's size
#   B. click on the TITLE BAR     -> falls back to the whole window (chrome is filtered)
#   C. control snapping OFF       -> clicking the same spot selects the whole window again
#
# ASCII only (see tools/probe/README.md).
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
Add-Type -Path (Join-Path $root 'tools\probe\ControlWindow.cs')
Add-Type -AssemblyName System.Drawing
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }
$work = Join-Path $tempRoot ("cuter-control-snap-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$saveDir = Join-Path $work 'shots'
New-Item -ItemType Directory -Force -Path $saveDir | Out-Null

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$titleShot = 'Cuter ' + [char]0x622A + [char]0x56FE
$script:blocked = $false

function Blocked-Message([string]$text) {
    $fg = [CuterInputProbe]::ForegroundProcess()
    if ($fg -and $fg -notmatch '^(cuter|powershell|pwsh|cmd|conhost|explorer|WindowsTerminal)$') {
        $script:blocked = $true
        Say "foreground app is '$fg' - [$([CuterInputProbe]::ForegroundTitle())]"
        return "SKIPPED_KEYBOARD_GRABBED: $text"
    }
    return $text
}

function Stop-Everything {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

trap {
    if ($script:blocked -or $_.Exception.Message -like 'SKIPPED_KEYBOARD_GRABBED*') {
        Write-Host "SKIPPED: $($_.Exception.Message -replace '^SKIPPED_KEYBOARD_GRABBED: ', '')"
    } else {
        Write-Host "FAILED: $($_.Exception.Message)"
    }
    try { [ControlWindow]::Stop() } catch { }
    Stop-Everything
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $configPath -Force }
    if ($script:blocked) {
        Write-Host 'RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive'
        exit 2
    }
    Write-Host 'RESULT: FAIL'
    exit 1
}

function Write-Config([bool]$snapControls) {
    $value = if ($snapControls) { 'true' } else { 'false' }
    New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
    Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true

[hotkeys]
capture = "Alt+C"

[capture]
hide_own_windows = true
delay_ms = 0
auto_detect_windows = true
snap_controls = $value
show_magnifier = false
include_cursor = false
remember_last_region = false
play_sound = false

[save]
dir = '$saveDir'
format = "png"
auto_save = true
ask_path_on_save = false
auto_copy = false
reveal_after_save = false

[history]
limit = 0

[ocr]
engine = "disabled"
"@
}

# ---------- target window ----------
Stop-Everything
$target = [ControlWindow]::Start('Cuter Probe Control Window', 260, 200, 560, 420)
if ($target -eq [IntPtr]::Zero) { throw 'could not create the control window' }
Start-Sleep -Milliseconds 800
# The app snaps to the DWM extended frame bounds, and the button may be DPI-scaled, so ask
# the window itself for both rects instead of assuming the creation constants.
$btn = @(([ControlWindow]::ButtonRect()) -split '\|')
$btnX = [int]$btn[0]; $btnY = [int]$btn[1]
$btnW = [int]$btn[2]; $btnH = [int]$btn[3]
$frame = @(([ControlWindow]::FrameRect()) -split '\|')
$fraX = [int]$frame[0]; $fraY = [int]$frame[1]
$winW = [int]$frame[2]; $winH = [int]$frame[3]
$winRect = @(([CuterInputProbe]::Describe($target)) -split '\|')
Say ("target frame {0}x{1} at ({2},{3}); button {4}x{5} at ({6},{7})" -f `
    $winW, $winH, $fraX, $fraY, $btnW, $btnH, $btnX, $btnY)

function Open-Overlay($app) {
    $overlay = [IntPtr]::Zero
    foreach ($attempt in 1..4) {
        [CuterInputProbe]::AltKey(0x43)
        foreach ($i in 1..25) {
            Start-Sleep -Milliseconds 200
            $overlay = [CuterInputProbe]::FindWindow([uint32]$app.Id, $titleShot)
            if ($overlay -ne [IntPtr]::Zero) { break }
        }
        if ($overlay -ne [IntPtr]::Zero) { break }
        [CuterInputProbe]::Key(0x1B)
        [void][CuterInputProbe]::FocusDesktop()
        Start-Sleep -Milliseconds 700
    }
    if ($overlay -eq [IntPtr]::Zero) { throw (Blocked-Message 'the overlay never appeared after Alt+C') }
}

# Newest saved PNG's size = the selection the app actually delivered.
function Save-Selection($app) {
    $n = @(Get-ChildItem $saveDir -Filter *.png -ErrorAction SilentlyContinue).Count
    [CuterInputProbe]::Key(0x0D)   # Enter = confirm (auto_save = true)
    foreach ($i in 1..40) {
        Start-Sleep -Milliseconds 200
        $files = @(Get-ChildItem $saveDir -Filter *.png -ErrorAction SilentlyContinue)
        if ($files.Count -gt $n) {
            $newest = $files | Sort-Object LastWriteTime -Descending | Select-Object -First 1
            $img = [System.Drawing.Image]::FromFile($newest.FullName)
            $size = "$($img.Width)x$($img.Height)"
            $img.Dispose()
            return $size
        }
    }
    throw 'no new saved file appeared (the overlay did not deliver)'
}

function Start-App {
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
    return $app
}

$fail = @()

# ---------- A. click ON the button: it must snap to the button ----------
Write-Config $true
[void][CuterInputProbe]::Activate($target)
Start-Sleep -Milliseconds 400
$app = Start-App
Open-Overlay $app
[void][CuterInputProbe]::MoveTo(($btnX + [int]($btnW / 2)), ($btnY + [int]($btnH / 2)))
Start-Sleep -Milliseconds 700      # let the throttled UIA probe answer
[CuterInputProbe]::Press()
Start-Sleep -Milliseconds 80
[CuterInputProbe]::Release()
Start-Sleep -Milliseconds 500
$got = Save-Selection $app
$want = "$($btnW)x$($btnH)"
Say "A. click on the button -> saved $got (button is $want)"
if ($got -ne $want) { $fail += "A: expected the button ($want), got $got" }

# ---------- B. click on the title bar: chrome is filtered, so the whole window ----------
Open-Overlay $app
[void][CuterInputProbe]::MoveTo(($fraX + 60), ($btnY + $btnH + 90))
Start-Sleep -Milliseconds 700
[CuterInputProbe]::Press()
Start-Sleep -Milliseconds 80
[CuterInputProbe]::Release()
Start-Sleep -Milliseconds 500
$got = Save-Selection $app
$want = "$($winW)x$($winH)"
Say "B. click on the title bar -> saved $got (window is $want)"
if ($got -ne $want) { $fail += "B: expected the whole window ($want), got $got" }

Stop-Everything
Start-Sleep -Milliseconds 500

# ---------- C. with control snapping OFF the same click selects the window ----------
Write-Config $false
$app = Start-App
Open-Overlay $app
[void][CuterInputProbe]::MoveTo(($btnX + [int]($btnW / 2)), ($btnY + [int]($btnH / 2)))
Start-Sleep -Milliseconds 700
[CuterInputProbe]::Press()
Start-Sleep -Milliseconds 80
[CuterInputProbe]::Release()
Start-Sleep -Milliseconds 500
$got = Save-Selection $app
$want = "$($winW)x$($winH)"
Say "C. snap_controls=false, click on the button -> saved $got (window is $want)"
if ($got -ne $want) { $fail += "C: with snapping off expected the window ($want), got $got" }

Stop-Everything
try { [ControlWindow]::Stop() } catch { }
Copy-Item $configBackup $configPath -Force

if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw 'control snapping e2e failed'
}
Say 'RESULT: PASS - snapping picked the button exactly, fell back to the window on chrome, and the switch turns it off'
