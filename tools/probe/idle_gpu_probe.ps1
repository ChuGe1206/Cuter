# Idle GPU usage: does the app burn GPU while it just sits in the tray?
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\idle_gpu_probe.ps1
#
# Why this probe exists: the user noticed "Cuter" sitting at 0.6% GPU in Task Manager while
# idle. That number is tiny but it is *persistent*, which means something keeps repainting
# forever. This probe pins the behaviour down as a number:
#   1. start the app with start_in_tray = true (nothing visible, no capture in flight);
#   2. let it settle, then sample \GPU Engine(*)\Utilization Percentage for OUR pid only;
#   3. report mean / max over the sampling window and assert the ceiling.
# It also reports CPU time delta, so "did we stop working?" is visible from two angles.
#
# ASCII only (see tools/probe/README.md).
param(
    [string]$Exe = '',
    [int]$Seconds = 15
)

$ErrorActionPreference = 'Stop'
function Say($m) { Write-Host $m }

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - build it first" }
$exe = (Resolve-Path $exe).Path
Say "probing $exe for $Seconds s of idling"

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$tempRoot = [System.IO.Path]::GetTempPath()
$configBackup = Join-Path $tempRoot ("cuter-idle-backup-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
Copy-Item $configPath $configBackup -Force

function Stop-App {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 700
}

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Stop-App
    if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL'
    exit 1
}

Stop-App
Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
theme = "auto"
language = "auto"
start_in_tray = true
close_to_tray = true
autostart = false

[capture]
hide_own_windows = true
auto_detect_windows = true
snap_controls = true
show_magnifier = false
include_cursor = false
remember_last_region = false
play_sound = false

[history]
limit = 0

[ocr]
engine = "disabled"
"@

$app = Start-Process $exe -PassThru
Say "app pid=$($app.Id); settling for 6 s"
Start-Sleep -Seconds 6
$app.Refresh()
if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
$cpuBefore = $app.TotalProcessorTime.TotalMilliseconds

# Sample the GPU engine counters that belong to our process. Instance names look like
#   pid_1234_luid_0x00000000_0x0000BEEF_phys_0_eng_0_engtype_3D
$samples = @()
$deadline = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $deadline) {
    $counter = Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction SilentlyContinue
    if ($counter) {
        $mine = $counter.CounterSamples | Where-Object { $_.InstanceName -like "pid_$($app.Id)_*" }
        if ($mine) { $samples += ($mine | Measure-Object -Property CookedValue -Sum).Sum }
    }
    Start-Sleep -Milliseconds 500
}
$app.Refresh()
$cpuAfter = $app.TotalProcessorTime.TotalMilliseconds
$cpuSeconds = ($cpuAfter - $cpuBefore) / 1000.0

Stop-App
Copy-Item $configBackup $configPath -Force

if ($samples.Count -eq 0) {
    Write-Host 'SKIPPED: no GPU engine counters for this process (driver/perf-counter unavailable)'
    Write-Host 'RESULT: SKIPPED'
    exit 2
}
$stats = $samples | Measure-Object -Average -Maximum
Say ("GPU samples: {0} over {1} s" -f $stats.Count, $Seconds)
Say ("GPU 3D+copy idle: mean {0:N3} % / max {1:N3} %" -f $stats.Average, $stats.Maximum)
Say ("CPU time consumed while idling: {0:N3} s" -f $cpuSeconds)

# Ceilings: an idle tray app should be at (almost) zero. Before the fix this sat at ~0.6 %.
$meanLimit = 0.25
$maxLimit = 2.0
if ($stats.Average -gt $meanLimit) {
    throw ("idle GPU mean {0:N3} % exceeds {1} % - something keeps repainting" -f $stats.Average, $meanLimit)
}
if ($stats.Maximum -gt $maxLimit) {
    throw ("idle GPU peak {0:N3} % exceeds {1} %" -f $stats.Maximum, $maxLimit)
}
Say 'RESULT: PASS - the app is idle in the tray without burning GPU'
