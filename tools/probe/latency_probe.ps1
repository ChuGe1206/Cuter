# End-to-end latency of "hotkey -> overlay actually painted on screen".
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\latency_probe.ps1
#   ... -Cycles 20 -Backend dxgi
#
# What it does: press Alt+C, wait for the overlay window, press Esc, repeat. The verdict
# comes from the app's own stage log, which is stamped from the moment the hotkey was
# handled ("request_at") - so this measures the whole chain:
#
#   handled -> capturing -> captured -> overlay window requested -> first frame painted
#
# The last line is the one users feel; until this round it did not exist, i.e. everything
# after "capture finished" was unmeasured. ASCII only (see tools/probe/README.md).
param(
    [string]$Exe = '',
    [int]$Cycles = 12,
    [string]$Backend = '',
    [int]$SettleMs = 400
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

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }
$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-latency-config-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
Copy-Item $configPath $configBackup -Force
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$script:blocked = $false

function Stop-AllCuter {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 600
}

trap {
    $msg = $_.Exception.Message
    if ($script:blocked) { Write-Host "SKIPPED: $msg" } else { Write-Host "FAILED: $msg" }
    Stop-AllCuter
    if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
    if ($script:blocked) { Write-Host 'RESULT: SKIPPED'; exit 2 }
    Write-Host 'RESULT: FAIL'; exit 1
}

# Write a COMPLETE minimal config instead of surgically editing the user's file.
#
# Editing was a real bug magnet: an earlier version inserted `backend` after the
# `[capture]` header with a regex, which once produced a **duplicate `[capture]` table**.
# The app then failed to parse the config, silently fell back to defaults (GDI), and the
# "dxgi" run was quietly measuring GDI. The assertion below now turns that into a failure.
$backendValue = if ($Backend) { $Backend } else { 'gdi' }
New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true

[hotkeys]
capture = "Alt+C"

[capture]
backend = "$backendValue"

[history]
limit = 0

[ocr]
engine = "disabled"
"@
Say "capture.backend = $backendValue"

Stop-AllCuter
$logBefore = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
$app = Start-Process $exe -PassThru
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 60000) {
    Start-Sleep -Milliseconds 200
    if ((Test-Path $logPath) -and ((Get-Item $logPath).Length -gt $logBefore)) { break }
    $app.Refresh()
    if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
}
# Keep the very first slice of the log: that is where the app writes its startup timeline
# (it is written before the first capture cycle, so the per-cycle slices below never see it).
Start-Sleep -Milliseconds 1500
$startupText = ''
if (Test-Path $logPath) {
    $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    $stream.Seek($logBefore, 'Begin') | Out-Null
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    $startupText = $reader.ReadToEnd()
    $reader.Dispose(); $stream.Dispose()
}

# Give the tray/DXGI warm-up time to settle so we measure the steady state, then make sure
# the DXGI backend really is up before the first cycle.
Start-Sleep -Seconds 3
$overlayTitle = 'Cuter ' + [char]0x622A + [char]0x56FE

[void][CuterInputProbe]::FocusDesktop()
Start-Sleep -Milliseconds 300

$samples = @()
for ($i = 1; $i -le $Cycles; $i++) {
    $offset = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    [CuterInputProbe]::AltKey(0x43)
    $overlay = [IntPtr]::Zero
    foreach ($try in 1..20) {
        Start-Sleep -Milliseconds 60
        $overlay = [CuterInputProbe]::FindWindow([uint32]$app.Id, $overlayTitle)
        # The overlay viewport is kept alive between captures, parked OFF the desktop at
        # (-32000,-32000) - and that window is still "visible" to IsWindowVisible. Requiring
        # it to be on the desktop is what makes this wait mean "the user can see it".
        if ($overlay -ne [IntPtr]::Zero -and [CuterInputProbe]::IsOnDesktop($overlay)) { break }
        $overlay = [IntPtr]::Zero
    }
    if ($overlay -eq [IntPtr]::Zero) {
        # one retry after dismissing whatever stole the keyboard
        [CuterInputProbe]::Key(0x1B)
        [void][CuterInputProbe]::FocusDesktop()
        Start-Sleep -Milliseconds 700
        [CuterInputProbe]::AltKey(0x43)
        foreach ($try in 1..20) {
            Start-Sleep -Milliseconds 60
            $overlay = [CuterInputProbe]::FindWindow([uint32]$app.Id, $overlayTitle)
            if ($overlay -ne [IntPtr]::Zero -and [CuterInputProbe]::IsOnDesktop($overlay)) { break }
            $overlay = [IntPtr]::Zero
        }
    }
    if ($overlay -eq [IntPtr]::Zero) {
        $fg = [CuterInputProbe]::ForegroundProcess()
        if ($fg -and $fg -notmatch '^(cuter|powershell|pwsh|cmd|conhost|explorer|WindowsTerminal)$') {
            $script:blocked = $true
            throw "no overlay appeared; foreground is '$fg' [$([CuterInputProbe]::ForegroundTitle())] (that program owns the keyboard)"
        }
        throw 'no overlay appeared after Alt+C'
    }
    Start-Sleep -Milliseconds $SettleMs
    [CuterInputProbe]::Key(0x1B)   # Esc closes the overlay
    Start-Sleep -Milliseconds $SettleMs

    $text = ''
    if (Test-Path $logPath) {
        $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
        try {
            $stream.Seek($offset, 'Begin') | Out-Null
            $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
            $text = $reader.ReadToEnd()
        } finally { $stream.Dispose() }
    }
    if ($text -match 'TOML parse error') {
        throw 'the app failed to parse the config it was given (see the log) - the measurement would be meaningless'
    }
    $visible = [regex]::Match($text, '\u906E\u7F69\u9996\u5E27\u53EF\u89C1\]\uFF1A\+(\d+) ms')
    $handled = [regex]::Match($text, '\u8FDB\u5165\u6355\u83B7\]\uFF1A\+(\d+) ms')
    $captured = [regex]::Match($text, '\u6355\u83B7\u5B8C\u6210\]\uFF1A\+(\d+) ms')
    $declared = [regex]::Match($text, '\u5DF2\u8BF7\u6C42\u906E\u7F69\u7A97\u53E3\]\uFF1A\+(\d+) ms')
    if (-not $visible.Success) { throw "cycle $i produced no 'overlay first frame' log line" }
    $row = [pscustomobject]@{
        Cycle    = $i
        Handled  = if ($handled.Success) { [int]$handled.Groups[1].Value } else { -1 }
        Captured = if ($captured.Success) { [int]$captured.Groups[1].Value } else { -1 }
        Declared = if ($declared.Success) { [int]$declared.Groups[1].Value } else { -1 }
        Visible  = [int]$visible.Groups[1].Value
        Hwnd     = $overlay.ToInt64()
    }
    $samples += $row
    Say ("cycle {0,2}: handled +{1} ms | captured +{2} ms | requested +{3} ms | FIRST FRAME +{4} ms | hwnd {5}" -f `
        $row.Cycle, $row.Handled, $row.Captured, $row.Declared, $row.Visible, $row.Hwnd)
}

Stop-AllCuter
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }

$values = $samples | ForEach-Object { $_.Visible } | Sort-Object
$tail = $samples | ForEach-Object { $_.Visible - $_.Captured } | Sort-Object
$median = $values[[int][math]::Floor($values.Count / 2)]
Say ''
# backend must travel with the number: GDI and DXGI differ by tens of milliseconds,
# so a bare "116 ms" is not a measurement anyone can compare against later.
Say ("hotkey -> overlay painted [{0}]: min {1} ms / median {2} ms / max {3} ms ({4} cycles)" -f `
    $backendValue, $values[0], $median, $values[-1], $values.Count)
Say ("of which capture-done -> painted (window + GL context + first upload): min {0} ms / median {1} ms / max {2} ms" -f `
    $tail[0], $tail[[int][math]::Floor($tail.Count / 2)], $tail[-1])

# ---------- startup timeline ----------
# The app logs "启动时间线[...]：+N ms" from process creation (GetProcessTimes) at each phase.
# Asserting it here keeps the instrumentation honest: if the logs stop appearing (or the
# numbers become absurd), this probe fails instead of quietly losing the data.
# The log text is Chinese; build the pattern from code points so this file stays pure ASCII
# (PowerShell 5.1 would parse a BOM-less script with Chinese as GBK and break).
$tag = [string]([char]0x542F) + [char]0x52A8 + [char]0x65F6 + [char]0x95F4 + [char]0x7EBF
$timelinePattern = [regex]::Escape($tag) + '\[([^\]]+)\].*\+(\d+) ms'
$timeline = @()
foreach ($line in ($startupText -split "`r?`n")) {
    if ($line -match $timelinePattern) { $timeline += $line }
}
if ($timeline.Count -eq 0) { throw 'no startup timeline lines in the log (instrumentation broken?)' }
# Print only the ASCII part of each line: the phase names are Chinese and the console code
# page would turn them into mojibake. The order matches the order the app logs them in
# (main -> config -> guard -> eframe -> hotkeys -> tray -> first frame).
$firstFrame = $null
$phase = 0
foreach ($line in $timeline) {
    if ($line -match '\+(\d+) ms') {
        $phase++
        $firstFrame = [int]$Matches[1]
        Say ('startup: phase {0} = +{1} ms (cumulative)' -f $phase, $firstFrame)
    }
}
if ($firstFrame -eq $null -or $firstFrame -lt 50 -or $firstFrame -gt 30000) {
    throw "implausible startup-to-first-frame time: $firstFrame ms"
}
Say ("process created -> first frame: {0} ms (this machine; antivirus scans can dominate it)" -f $firstFrame)
Say 'RESULT: PASS'
