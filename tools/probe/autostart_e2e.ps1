# autostart end-to-end test: does `general.autostart` really drive HKCU\...\Run?
#
# ASCII only (see tools/probe/README.md).
#
# Four cases, all against the REAL registry (the whole point is that static reading of
# the code cannot tell whether the switch works - see docs/config-audit.md step 5):
#
#   A. config off + somebody else's entry    -> the entry must be left ALONE
#      (a portable copy must never delete the installed copy's autostart)
#   B. config on                            -> the entry appears, pointing at our exe
#      with `--tray`
#   C. config on + stale entry (moved exe)  -> the entry is corrected on startup
#   D. config off + our own entry           -> the entry is removed
#
# It backs up the existing value (if any) and restores it in `finally`.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\autostart_e2e.ps1
#   ... -Exe .\target\release\cuter.exe
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

# Scratch files MUST be absolute. If the temp directory cannot be resolved, `Join-Path`
# silently returns a RELATIVE path and the backup lands in the repository - that really
# happened once and the stray file got committed. Ask the framework instead of trusting
# an environment variable, and fail loudly if it is not absolute.
$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-autostart-config-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$logDir = Join-Path $env:APPDATA 'Cuter\logs'
# The app always logs the font it picked at startup, so "this file grew" is positive
# proof that a real instance ran our startup code (see Start-And-Kill).
$logPath = Join-Path $logDir 'cuter.log'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$valueName = 'Cuter'

function Get-Autostart {
    try { return [string](Get-ItemProperty -Path $runKey -Name $valueName -ErrorAction Stop).$valueName }
    catch { return $null }
}
function Set-Autostart([string]$value) {
    Set-ItemProperty -Path $runKey -Name $valueName -Value $value -Force
}
function Remove-Autostart {
    Remove-ItemProperty -Path $runKey -Name $valueName -ErrorAction SilentlyContinue
}

# ---------- 0. back up + clean slate ----------
$original = Get-Autostart
if ($null -ne $original) {
    Say "backing up existing autostart entry: $original"
    $backupValue = Join-Path $tempRoot 'cuter-autostart-backup.txt'
    Set-Content -Path $backupValue -Value $original -Encoding UTF8
} else {
    $backupValue = $null
    Say "no existing autostart entry"
}
Remove-Autostart
if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force }

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    # 收尾必须自己拼路径：脚本前半段抛错时上面的变量可能还没赋值。
    $cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $cfg -Force }
    $bk = Join-Path ([System.IO.Path]::GetTempPath()) 'cuter-autostart-backup.txt'
    if (Test-Path $bk) { Set-Autostart (Get-Content $bk -Raw).Trim() } else { Remove-Autostart }
    exit 1
}

# Signature of the log directory (names + sizes). Comparing this before/after a start
# proves the app really ran, and it survives log rotation (a rotated file changes the set).
function Get-LogSignature {
    if (-not (Test-Path $logDir)) { return '' }
    return ((Get-ChildItem $logDir -File | Sort-Object Name |
        ForEach-Object { "$($_.Name):$($_.Length)" }) -join '|')
}

function Write-Config([bool]$autostart) {
    $on = if ($autostart) { 'true' } else { 'false' }
    New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
    Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true
close_to_tray = true
autostart = $on

[capture]
hide_own_windows = true
delay_ms = 0
include_cursor = false
auto_detect_windows = false
show_magnifier = false

[ocr]
engine = "disabled"
"@
}

# Start the app, wait until it has really run our startup code, then kill it hard
# (killing instead of closing on purpose: we want to observe what STARTUP did, not what
# a graceful exit might do).
#
# ⚠️ Four guards, all learned the hard way ("the app never ran" masquerades as
# "the app ran and did nothing", and case A passes either way):
#   1. refuse to start when another instance exists - the single-instance guard would
#      hand our start over to it, our process would just exit, and nothing would be logged;
#   2. wait for the app's own log to change (it logs the font it picked, and that happens
#      AFTER the autostart reconciliation) - positive proof that a real instance ran;
#      with a timeout, because a cold start under Defender can take seconds;
#   3. fail if the process exits before that happened;
#   4. wait until it is really gone before returning, otherwise the next case starts while
#      this one is still shutting down.
# Per-start stdout/stderr files are kept so a handover message stays visible.
function Start-And-Kill([int]$seconds = 2, [string]$tag = 'case') {
    $stale = @(Get-Process cuter -ErrorAction SilentlyContinue)
    if ($stale.Count -gt 0) {
        throw "another cuter instance is running ($($stale.Count)) - it would swallow this start"
    }
    $before = Get-LogSignature
    $out = Join-Path $tempRoot "cuter-autostart-$tag.out"
    $err = Join-Path $tempRoot "cuter-autostart-$tag.err"

    $p = Start-Process $exe -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
    $started = $false
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # Two independent "a real instance ran" signals, either is enough:
    #   * the app's stderr file has content (it always logs the font it picked at startup);
    #   * the log directory's signature changed (covers CUTER_LOG filtering stderr away).
    # The window is deliberately generous: a cold process start on a loaded machine can
    # take tens of seconds (Defender scanning an 8 MB exe), and this probe is not a
    # startup-time benchmark.
    while ($sw.ElapsedMilliseconds -lt 60000) {
        if ((Get-LogSignature) -ne $before) { $started = $true; break }
        if ((Test-Path $err) -and (Get-Content $err -Raw -ErrorAction SilentlyContinue)) {
            $started = $true
            break
        }
        $p.Refresh()
        if ($p.HasExited) {
            $stdout = if (Test-Path $out) { (Get-Content $out -Raw) } else { '' }
            throw "the app exited before doing any work (exit code $($p.ExitCode)): $stdout"
        }
        Start-Sleep -Milliseconds 200
    }
    if (-not $started) {
        Say ("  [diag] before=[{0}]" -f $before)
        Say ("  [diag] now   =[{0}]" -f (Get-LogSignature))
        Say ("  [diag] elapsed={0} ms exited={1}" -f $sw.ElapsedMilliseconds, $p.HasExited)
        throw 'the app never ran our startup code within 60 s'
    }
    Say ("  [diag] app reached its startup log after {0} ms" -f $sw.ElapsedMilliseconds)
    if ($seconds -gt 0) { Start-Sleep -Seconds $seconds }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 100
        if (-not (Get-Process cuter -ErrorAction SilentlyContinue)) { break }
    }
    if (Get-Process cuter -ErrorAction SilentlyContinue) {
        throw 'a cuter process is still running after the kill - the next case would be swallowed'
    }
}

$fail = @()
$expected = '"' + $exe + '" --tray'

# ---------- A. off + somebody else's entry: leave it alone ----------
$other = '"D:\Somewhere\Else\cuter.exe" --tray'
Set-Autostart $other
Write-Config $false
Start-And-Kill -tag 'a'
$got = Get-Autostart
Say "A. config off, foreign entry -> '$got'"
if ($got -ne $other) { $fail += "A: a foreign autostart entry was touched (got '$got')" } else { Say "   PASS: left alone" }

# ---------- B. on: the entry appears and points at us ----------
Remove-Autostart
Write-Config $true
Start-And-Kill -tag 'b'
$got = Get-Autostart
Say "B. config on -> '$got'"
if ($got -ne $expected) { $fail += "B: expected '$expected', got '$got'" } else { Say "   PASS: written with --tray" }

# ---------- C. stale entry (the exe moved): corrected on startup ----------
Set-Autostart '"D:\Old\Location\cuter.exe" --tray'
Start-And-Kill -tag 'c'
$got = Get-Autostart
Say "C. stale entry with config on -> '$got'"
if ($got -ne $expected) { $fail += "C: stale entry was not healed (got '$got')" } else { Say "   PASS: healed" }

# ---------- D. off + our own entry: removed ----------
Write-Config $false
Start-And-Kill -tag 'd'
$got = Get-Autostart
Say "D. config off, our entry -> '$got'"
if ($null -ne $got) { $fail += "D: our own entry was not removed (got '$got')" } else { Say "   PASS: removed" }

# ---------- cleanup ----------
if ($backupValue -and (Test-Path $backupValue)) {
    Set-Autostart (Get-Content $backupValue -Raw).Trim()
    Remove-Item $backupValue -Force -ErrorAction SilentlyContinue
} else {
    Remove-Autostart
}
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }

if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw 'autostart e2e failed'
}
Say 'RESULT: PASS - general.autostart drives HKCU\...\Run in all four cases'
