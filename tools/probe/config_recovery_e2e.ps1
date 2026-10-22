# config_recovery_e2e.ps1 - a broken config must be backed up, reported, and survivable.
#
# 0.1.59 made the app say "config could not be read, defaults are in use, the original
# went here" instead of silently resetting every setting, and made the backup name
# timestamped so a second failure cannot overwrite the first one's copy.
#
# Unit tests cover the loader. This probe covers the wiring: the app must start with a
# broken config, keep the broken bytes in a timestamped backup, log the warning, and
# still be a normal running instance afterwards.
#
# ASCII only (see tools/probe/README.md).

param(
    [string]$Exe = '',
    [int]$WaitSeconds = 12
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
Add-Type -Path (Join-Path $PSScriptRoot 'CuterInputProbe.cs')
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null

if (-not $Exe) { $Exe = Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $Exe)) { throw "missing $Exe - build it first" }

function Say($m) { Write-Host $m }

$cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
$cfgDir = Split-Path -Parent $cfg
$backup = "$cfg.recovery-probe-backup"
New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
if (Test-Path $backup) { Remove-Item $backup -Force }
if (Test-Path $cfg) { Copy-Item $cfg $backup -Force }

$log = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$broken = "# this file is deliberately broken on purpose`nnot = valid [ toml `n"

try {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 800

    # Snapshot the existing backups so we can tell the new one apart.
    $script:before = @(Get-ChildItem $cfgDir -Filter 'config.toml.broken-*' -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty Name)
    $logMark = if (Test-Path $log) { (Get-Item $log).Length } else { 0 }

    Set-Content -Path $cfg -Value $broken -Encoding UTF8 -NoNewline
    Say "wrote a broken config ($((Get-Item $cfg).Length) bytes) and starting the app"

    $app = Start-Process $Exe -PassThru
    # A broken config means default values, and the default is start_in_tray = true: the app
    # comes up as a tray instance with no window. So wait for the process to settle, then
    # ask it for its settings window the way a user would (the app hands over to the
    # already-running instance, which is also proof it is alive and responsive).
    $started = $false
    foreach ($i in 1..($WaitSeconds * 4)) {
        Start-Sleep -Milliseconds 250
        if ($app.HasExited) { break }
        $started = $true
        break
    }
    if (-not $started) { throw "RESULT: FAIL - the app exited instead of falling back to defaults" }
    Start-Sleep -Seconds 2
    & $Exe | Out-Null
    $window = $false
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 250
        if ([CuterInputProbe]::FindWindow([uint32]$app.Id, 'Cuter') -ne [IntPtr]::Zero) { $window = $true; break }
    }
    if (-not $window) { throw 'RESULT: FAIL - the app never opened its settings window' }
    Say 'the app started with a broken config and opened its settings window'

    Start-Sleep -Seconds 2

    $after = @(Get-ChildItem $cfgDir -Filter 'config.toml.broken-*' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending)
    if ($after.Count -le $script:before.Count) {
        throw "RESULT: FAIL - no config.toml.broken-* backup was written ($($script:before.Count) before, $($after.Count) after)"
    }
    $newest = $after[0]
    $kept = Get-Content $newest.FullName -Raw -Encoding UTF8
    if ($kept -notmatch 'deliberately broken') {
        throw "RESULT: FAIL - the backup does not hold the broken file's bytes: $($newest.Name)"
    }
    Say ("backup written: {0} ({1} bytes, contents preserved)" -f $newest.Name, $newest.Length)

    # The live config must not still be the broken text: the app falls back to defaults and
    # rewrites it. Whether it exists yet depends on whether anything was persisted, so only
    # assert that it is not the broken bytes.
    if (Test-Path $cfg) {
        $live = Get-Content $cfg -Raw -Encoding UTF8
        if ($live -match 'deliberately broken') {
            throw 'RESULT: FAIL - the broken file was left in place as the live config'
        }
        Say 'the live config was replaced with defaults'
    } else {
        Say 'no live config yet (defaults in memory only)'
    }

    $stream = [System.IO.File]::Open($log, 'Open', 'Read', 'ReadWrite')
    $stream.Seek($logMark, 'Begin') | Out-Null
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    $fresh = $reader.ReadToEnd()
    $reader.Dispose()
    $stream.Dispose()
    # The log text is Chinese. Build the marker from code points so this file stays ASCII
    # (a BOM-less UTF-8 .ps1 with Chinese comments is parsed as GBK by PowerShell 5.1).
    $beiFen = '' + [char]0x5907 + [char]0x4EFD   # "backed up"
    if ($fresh -notmatch $beiFen) {
        throw 'RESULT: FAIL - the app did not log that it backed up and fell back'
    }
    Say 'the app logged the fallback and the backup path'

    Say 'RESULT: PASS - a broken config is backed up (timestamped), reported, and survivable'
    exit 0
}
catch {
    Write-Host $_.Exception.Message
    exit 1
}
finally {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    # Do not leave the artifacts around: the probe deliberately breaks the config, so the
    # user's directory must end up exactly as it started (settings restored, no extra
    # .broken-* files from this run).
    foreach ($extra in (Get-ChildItem $cfgDir -Filter 'config.toml.broken-*' -ErrorAction SilentlyContinue |
            Where-Object { $before -notcontains $_.Name })) {
        Remove-Item $extra.FullName -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path $backup) { Copy-Item $backup $cfg -Force; Remove-Item $backup -Force }
}
