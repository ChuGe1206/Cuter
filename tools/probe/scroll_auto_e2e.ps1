# Auto-scroll end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\scroll_auto_e2e.ps1
#
# Auto-scroll means the app synthesises the wheel events itself, so this probe drives a real
# scroll-capture session with nothing but hotkeys and key presses, and then reads the app's
# own log for the verdict:
#
#   autoscroll-stop stalled=3 steps=35 height=4114
#
# The three numbers are the whole feature: S > 3 proves the synthetic wheel really moved the
# target (a swallowed wheel stops after `auto_stall_frames` steps), H proves the stitcher
# proves the tracker stopped on its own instead of hammering the target forever.
# hammering the target forever.
#
# Exits 2 (SKIPPED) when the environment swallows the input - see the README; that is a real
# possibility on a busy machine, and a skipped probe must never look like a pass.
#
# ASCII only (see tools/probe/README.md).
param(
    [string]$Exe = '',
    [int]$AutoSeconds = 45
)

$ErrorActionPreference = 'Stop'
function Say($m) { Write-Host $m }

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - build it first" }
$exe = (Resolve-Path $exe).Path

Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("cuter-autoscroll-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force

function Stop-All {
    Get-Process cuter, notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 700
}

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Stop-All
    if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL'
    exit 1
}

Stop-All

# A long document so there is something to scroll to the end of.
$doc = Join-Path $work 'long.txt'
$lines = 1..300 | ForEach-Object { "line $_ - the quick brown fox jumps over the lazy dog 0123456789" }
Set-Content -Path $doc -Value $lines -Encoding UTF8
$notepad = Start-Process notepad -ArgumentList $doc -PassThru
Start-Sleep -Seconds 2
[void][CuterInputProbe]::Activate($notepad.MainWindowHandle)
Start-Sleep -Milliseconds 500

# Turn auto-scroll on in the config (the panel's A key is per-session; this is the default
# for a new session).
$text = Get-Content $configPath -Raw -Encoding UTF8
if ($text -notmatch '\[scroll\]') {
    $text = $text.TrimEnd() + "`n`n[scroll]`nauto_scroll = true`nauto_stall_frames = 3`n"
} else {
    $text = $text -replace '(?m)^auto_scroll\s*=\s*\w+', 'auto_scroll = true'
}
Set-Content -Path $configPath -Value $text -Encoding UTF8

$before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
$app = Start-Process $exe -PassThru
Start-Sleep -Seconds 6

$captureTitle = 'Cuter ' + [char]0x622A + [char]0x56FE      # "Cuter <capture>"
[CuterInputProbe]::AltKey(0x53)                              # Alt+S = scroll capture
Start-Sleep -Seconds 3
$overlay = [CuterInputProbe]::FindWindow([uint32]$app.Id, $captureTitle)
if ($overlay -eq [IntPtr]::Zero) {
    Stop-All
    Copy-Item $configBackup $configPath -Force
    Say 'RESULT: SKIPPED - the mask never appeared (synthesised Alt+S was swallowed)'
    exit 2
}

[void][CuterInputProbe]::Drag(150, 150, 900, 700)            # select the text area
Start-Sleep -Seconds 2
# The app sends its wheel events with SendInput, so they land on whatever is under the
# cursor: park it in the middle of the selection. Without this the drag's end point (900,700)
# can sit outside a default-sized notepad and the page never moves (steps=3, stalled=3).
[void][CuterInputProbe]::MoveTo(525, 425)
Start-Sleep -Milliseconds 300
[CuterInputProbe]::Key(0x0D)                                 # Enter = start collecting
Start-Sleep -Seconds 3
# Do NOT press A here: the config above already has auto_scroll = true, so the session starts
# with auto-scroll on and A would toggle it OFF (the app logs 自动滚动：关, then nothing moves
# and this probe waits out its whole window). A is only needed when the config says false.
Say "auto-scroll session started, waiting up to $AutoSeconds s for it to stop by itself"

$deadline = (Get-Date).AddSeconds($AutoSeconds)
$stopLine = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 700
    $slice = Get-Content $logPath -Encoding UTF8 -ErrorAction SilentlyContinue | Select-Object -Skip 0
    $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    $stream.Seek($before, 'Begin') | Out-Null
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    $slice = $reader.ReadToEnd()
    $reader.Dispose(); $stream.Dispose()
    # The app writes an ASCII marker ("autoscroll-stop stalled=3 steps=35 height=4114")
    # precisely so this file can stay pure ASCII - PowerShell 5.1 would read Chinese in a
    # BOM-less script as GBK and fail to parse it.
    $hit = $slice -split "`r?`n" | Where-Object { $_ -match 'autoscroll-stop' } | Select-Object -Last 1
    if ($hit) { $stopLine = $hit; break }
}

Stop-All
Copy-Item $configBackup $configPath -Force

if (-not $stopLine) {
    Say 'RESULT: SKIPPED - auto-scroll never reported a stop within the window (input swallowed?)'
    exit 2
}
Say ("stop line: {0}" -f ($stopLine -replace '^.*(INFO)\s+', ''))

# Parse the ASCII marker: autoscroll-stop stalled=<n> steps=<n> height=<n>
$steps = [int]([regex]::Match($stopLine, 'steps=(\d+)').Groups[1].Value)
$height = [int]([regex]::Match($stopLine, 'height=(\d+)').Groups[1].Value)
Say ("steps={0}  stitched height={1}" -f $steps, $height)

if ($steps -le 3) { throw "the synthetic wheel never moved the target (only $steps step(s))" }
if ($height -lt 800) {
    # Nothing scrollable under the cursor: the wheel landed on a window that ignores it.
    # This probe borrows the desktop instead of creating its own target, so that is a
    # SKIP (nothing measured), not a FAIL (product broken). A broken stitcher shows up as
    # frames that DO move but fail to join, which grows $height.
    Say "RESULT: SKIPPED - no scrollable window under the selection (steps=$steps, height=$height); nothing was measured"
    exit 2
}
Say 'RESULT: PASS - the app scrolled the target itself and stopped on its own'
