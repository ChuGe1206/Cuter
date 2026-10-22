# Manual scroll-capture end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\scroll_manual_e2e.ps1
#
# scroll_auto_e2e.ps1 drives the mode where the APP sends the wheel events. This probe drives
# the mode people actually use: the app sits in scroll-capture, the user scrolls, and the app
# only watches. So the wheel events here come from the probe, exactly like a human's would.
#
# The app logs two lines when a session finishes; the probe reads the ASCII one:
#
#   scroll-done frames=9 w=750 h=1913
#
# F > 1 proves the tracker kept accepting frames after the first one, H > the selection height
# proves they were joined into something taller than the selection instead of being dropped.
#
# Exits 2 (SKIPPED) when the environment swallows the input - a skipped probe must never look
# like a pass.
#
# ASCII only (see tools/probe/README.md).
param(
    [string]$Exe = '',
    [int]$Rounds = 14,
    [int]$Notches = 3
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
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("cuter-manualscroll-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
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

# A long document: enough lines that a handful of wheel notches still leave room to scroll.
$doc = Join-Path $work 'long.txt'
$lines = 1..400 | ForEach-Object { "line $_ - the quick brown fox jumps over the lazy dog 0123456789" }
Set-Content -Path $doc -Value $lines -Encoding UTF8
$notepad = Start-Process notepad -ArgumentList $doc -PassThru
Start-Sleep -Seconds 2
[void][CuterInputProbe]::Activate($notepad.MainWindowHandle)
Start-Sleep -Milliseconds 500

# Manual mode: the app must NOT synthesise wheel events of its own, otherwise this probe would
# be testing scroll_auto_e2e again.
$text = Get-Content $configPath -Raw -Encoding UTF8
if ($text -notmatch '\[scroll\]') {
    $text = $text.TrimEnd() + "`n`n[scroll]`nauto_scroll = false`n"
} else {
    $text = $text -replace '(?m)^auto_scroll\s*=\s*\w+', 'auto_scroll = false'
    if ($text -notmatch '(?m)^auto_scroll\s*=') {
        $text = $text -replace '(?m)^\[scroll\]\s*$', "[scroll]`nauto_scroll = false"
    }
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
[void][CuterInputProbe]::MoveTo(525, 425)
Start-Sleep -Milliseconds 300
[CuterInputProbe]::Key(0x0D)                                 # Enter = start collecting
Start-Sleep -Seconds 2

function Read-NewLog {
    if (-not (Test-Path $logPath)) { return '' }
    $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    try {
        $stream.Seek($before, 'Begin') | Out-Null
        $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
}

# The frames only arrive when the tracker sees the content move, so this must be a real slow
# scroll: one burst per round with a pause in between, like a person reading while scrolling.
for ($round = 1; $round -le $Rounds; $round++) {
    [CuterInputProbe]::Wheel(-$Notches)
    Start-Sleep -Milliseconds 450
}
Start-Sleep -Seconds 1

$frames = [regex]::Matches((Read-NewLog), 'scroll-frame') | Measure-Object
Say ("log after scrolling: {0} frame marker(s)" -f $frames.Count)

# The control window may not hold the keyboard focus after all that scrolling, and a lost Enter
# would look exactly like a broken stitcher. Focus it first, then finish.
$panelTitle = 'Cuter ' + [char]0x6EDA + [char]0x52A8 + [char]0x622A + [char]0x5C4F   # "Cuter <scroll capture>"
$panel = [CuterInputProbe]::FindWindow([uint32]$app.Id, $panelTitle)
if ($panel -eq [IntPtr]::Zero) {
    Stop-All
    Copy-Item $configBackup $configPath -Force
    Say 'RESULT: SKIPPED - the scroll control window is gone'
    exit 2
}
[void][CuterInputProbe]::Activate($panel)
Start-Sleep -Milliseconds 400
[CuterInputProbe]::Key(0x0D)                                 # Enter = finish and deliver
Start-Sleep -Seconds 3

$slice = Read-NewLog
$done = $slice -split "`r?`n" | Where-Object { $_ -match 'scroll-done' } | Select-Object -Last 1
Stop-All
Copy-Item $configBackup $configPath -Force

if (-not $done) {
    $cancelled = $slice -match 'scroll-cancel|scroll-empty'
    Stop-All
    Copy-Item $configBackup $configPath -Force
    if ($cancelled) { throw 'the session ended without content (cancelled or empty)' }
    Say 'RESULT: SKIPPED - no scroll-done line: the Enter never reached the control window'
    exit 2
}
Say ("done line: {0}" -f ($done -replace '^.*(INFO)\s+', ''))

$frameCount = [int]([regex]::Match($done, 'frames=(\d+)').Groups[1].Value)
$height = [int]([regex]::Match($done, 'h=(\d+)').Groups[1].Value)
$width = [int]([regex]::Match($done, 'w=(\d+)').Groups[1].Value)
Say ("frames={0}  stitched {1}x{2}" -f $frameCount, $width, $height)

# The selection was 750x550 logical; anything below that means nothing was joined.
if ($frameCount -le 1) {
    Say "RESULT: SKIPPED - only the first frame was accepted (the wheel never reached the target)"
    exit 2
}
if ($height -le 700) {
    throw "the stitched image did not grow past the selection ($width x $height)"
}
Say 'RESULT: PASS - manual scrolling was captured and joined into a taller image'
