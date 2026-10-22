# Run every end-to-end probe and print one summary table.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\run_all.ps1
#   ... -Only pin              # only probes whose name contains "pin"
#   ... -Quick                 # skip the slow ones (perf / latency / leak / idle GPU)
#
# Exit codes follow the house convention used by the individual probes:
#   0 = PASS, 2 = SKIPPED (the environment could not run it honestly - e.g. another program
#   owns the keyboard), anything else = FAIL. This script exits 0 only when nothing failed:
#   a skipped probe is NOT a failure, but it is printed loudly so it cannot be mistaken for
#   a pass.
#
# Why it exists: every probe is meant to be run after every meaningful change, and "I forgot
# to run X" has already cost this project real bugs (a probe stayed broken for three rounds
# because nobody ran it). One command, one table.
#
# ASCII only (see tools/probe/README.md).
param(
    [string]$Only = '',
    [switch]$Quick,
    [int]$TimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$slow = @('perf_probe', 'latency_probe', 'leak_probe', 'idle_gpu_probe', 'history_e2e', 'installer_e2e')
$probes = Get-ChildItem (Join-Path $root 'tools\probe') -Filter '*.ps1' |
    Where-Object { $_.Name -match '(_e2e|_probe)\.ps1$' } |
    Sort-Object Name
if ($Only) { $probes = $probes | Where-Object { $_.Name -like "*$Only*" } }
if ($Quick) { $probes = $probes | Where-Object { $slow -notcontains ($_.BaseName) } }
if (-not $probes) { throw "no probes matched (Only='$Only' Quick=$Quick)" }

Write-Host ("running {0} probe(s){1}" -f $probes.Count, $(if ($Quick) { ' (quick)' } else { '' }))
Write-Host ''

# One probe run. Reaps the shared instance and settles first: they all drive the SAME
# installed app and the same user config (%APPDATA%\Cuter\config.toml), so a previous
# probe's app still shutting down - or a config it left behind - makes the next one fail
# in ways that look like regressions but are pure interference.
function Invoke-Probe($probe, [string]$logPath) {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 1500
    $job = Start-Job -ScriptBlock {
        param($script, $log)
        & powershell -NoProfile -ExecutionPolicy Bypass -File $script *> $log
        $LASTEXITCODE
    } -ArgumentList $probe.FullName, $logPath
    $code = $null
    if (Wait-Job $job -Timeout $TimeoutSeconds) {
        $code = Receive-Job $job
    } else {
        Stop-Job $job
        $code = -1
    }
    Remove-Job $job -Force
    return $code
}

$results = @()
foreach ($probe in $probes) {
    $log = Join-Path $env:TEMP ("cuter-probe-{0}.log" -f $probe.BaseName)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $code = Invoke-Probe $probe $log
    $retried = $false
    if ($code -ne 0 -and $code -ne 2) {
        # These are UI probes: one class of failure is "the previous probe just left the
        # focus / clipboard / foreground window in a weird state". Retry once, but say so
        # in the table - "passed on retry" is not the same as "passed".
        $retried = $true
        Copy-Item $log "$log.first" -Force -ErrorAction SilentlyContinue
        Write-Host ("         {0} failed (code {1}), retrying once ..." -f $probe.BaseName, $code)
        $code = Invoke-Probe $probe $log
    }
    $sw.Stop()

    $verdict = switch ($code) {
        0 { if ($retried) { 'PASS*' } else { 'PASS' } }
        2 { 'SKIP' }
        -1 { 'TIMEOUT' }
        default { "FAIL($code)" }
    }
    $detail = ''
    if (Test-Path $log) {
        $line = Get-Content $log -Encoding UTF8 | Select-String 'RESULT' | Select-Object -Last 1
        if ($line) { $detail = ($line.Line -replace '^RESULT:\s*', '') }
    }
    if ($retried -and $code -eq 0) {
        $detail = "passed only on retry (first log: $log.first) - $detail"
    } elseif ($retried) {
        $detail = "failed on both attempts (first log: $log.first) - $detail"
    }
    $results += [pscustomobject]@{
        Probe = $probe.BaseName; Verdict = $verdict; Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1); Detail = $detail
    }
    Write-Host ("{0,-8} {1,-28} {2,5:N1}s  {3}" -f $verdict, $probe.BaseName, $sw.Elapsed.TotalSeconds, $detail)
}

Write-Host ''
$passed = @($results | Where-Object { $_.Verdict -like 'PASS*' }).Count
$retryPassed = @($results | Where-Object { $_.Verdict -eq 'PASS*' }).Count
$skipped = @($results | Where-Object { $_.Verdict -eq 'SKIP' }).Count
$failed = @($results | Where-Object { $_.Verdict -notlike 'PASS*' -and $_.Verdict -ne 'SKIP' })
Write-Host ("{0} passed, {1} skipped, {2} failed (of {3})" -f $passed, $skipped, $failed.Count, $results.Count)
if ($retryPassed -gt 0) {
    Write-Host ("  note: {0} of the passes needed a retry - they are not stable, see PASS* rows" -f $retryPassed)
}
foreach ($f in $failed) {
    Write-Host ("  FAILED: {0} -> {1}" -f $f.Probe, $f.Detail)
}
if ($failed.Count -gt 0) { exit 1 }
Write-Host 'RESULT: PASS - every probe that could run, passed'
