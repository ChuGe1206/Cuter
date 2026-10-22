# idle_trim_probe.ps1 - the app returns its working set to the system when idle in tray.
#
# Why this exists: the trim is invisible in every other probe, because they all keep a
# window open (and then the font is loaded and the app is busy). A behaviour that only
# shows up after 30 quiet seconds needs a probe that is willing to wait 30 quiet seconds,
# otherwise it will rot silently.
#
# What it asserts:
#   1. with no window of ours on screen, the working set drops to a small fraction of the
#      idle baseline after the threshold (30 s) has passed;
#   2. the private commit does NOT drop - the trim hands pages back, it does not free memory
#      (so nobody may claim "we saved 50 MB");
#   3. a hotkey after the trim still works (the overlay appears), i.e. the pages come back.
#
# ASCII only (see tools/probe/README.md).

param(
    [string]$Exe = '',
    [int]$BaselineSeconds = 8,
    [int]$TrimSeconds = 40,
    [double]$MaxFraction = 0.5
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
$probeRoot = $PSScriptRoot
Add-Type -Path (Join-Path $probeRoot 'CuterInputProbe.cs')
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null

if (-not $Exe) { $Exe = Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $Exe)) { throw "missing $Exe - build it first" }

$cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
$backup = "$cfg.idle-trim-backup"
if (Test-Path $backup) { Remove-Item $backup -Force }
if (Test-Path $cfg) { Copy-Item $cfg $backup -Force }

function Get-Stat([int]$id) {
    $p = Get-Process -Id $id -ErrorAction Stop
    [pscustomobject]@{
        WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    }
}

$app = $null
try {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 800

    # A complete minimal config: start straight into the tray, so no window (and therefore
    # no font, no GL surface for a visible window) is ever created.
    New-Item -ItemType Directory -Force -Path (Split-Path $cfg) | Out-Null
    @"
[general]
theme = "dark"
start_in_tray = true
[hotkeys]
capture = "Alt+C"
[capture]
backend = "gdi"
"@ | Set-Content -Path $cfg -Encoding UTF8

    $app = Start-Process $Exe -PassThru
    Start-Sleep -Seconds $BaselineSeconds
    $before = Get-Stat $app.Id
    Write-Host ("tray baseline {0,6:N1} MB WS / {1,6:N1} MB private" -f $before.WorkingSetMB, $before.PrivateMB)

    Start-Sleep -Seconds $TrimSeconds
    $after = Get-Stat $app.Id
    Write-Host ("after {0} s idle {1,6:N1} MB WS / {2,6:N1} MB private" -f $TrimSeconds, $after.WorkingSetMB, $after.PrivateMB)

    $failed = $false
    if ($after.WorkingSetMB -ge ($before.WorkingSetMB * $MaxFraction)) {
        Write-Host ("FAILED: the working set stayed at {0} MB (baseline {1} MB, expected below {2} MB)" -f `
            $after.WorkingSetMB, $before.WorkingSetMB, [math]::Round($before.WorkingSetMB * $MaxFraction, 1))
        $failed = $true
    }
    if ($after.PrivateMB -lt ($before.PrivateMB * 0.8)) {
        Write-Host ("FAILED: the private commit also shrank ({0} MB -> {1} MB) - the trim must only hand pages back" -f `
            $before.PrivateMB, $after.PrivateMB)
        $failed = $true
    }
    if ($failed) { throw 'RESULT: FAIL - the idle trim did not behave as documented' }

    # The pages have to come back: the hotkey must still produce an overlay.
    [CuterInputProbe]::KeyDown(0x12)
    [CuterInputProbe]::Key(0x43)   # Alt+C
    [CuterInputProbe]::KeyUp(0x12)
    $overlay = [IntPtr]::Zero
    foreach ($i in 1..40) {
        Start-Sleep -Milliseconds 250
        $overlay = [CuterInputProbe]::FindWindow([uint32]$app.Id, 'Cuter')
        if ($overlay -ne [IntPtr]::Zero) { break }
    }
    if ($overlay -eq [IntPtr]::Zero) { throw 'RESULT: FAIL - the hotkey produced no overlay after the trim' }
    [CuterInputProbe]::Key(0x1B)   # Esc closes the overlay
    Start-Sleep -Milliseconds 500

    Write-Host ("trimmed {0:N1} MB -> {1:N1} MB of working set, private commit unchanged at {2:N1} MB, hotkey still works" -f `
        $before.WorkingSetMB, $after.WorkingSetMB, $after.PrivateMB)
    Write-Host 'RESULT: PASS - the app hands its working set back when idle in the tray'
    exit 0
}
catch {
    Write-Host $_.Exception.Message
    exit 1
}
finally {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    if (Test-Path $backup) { Copy-Item $backup $cfg -Force; Remove-Item $backup -Force }
}
