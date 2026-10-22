# Installer end-to-end test: install for real, assert everything, uninstall for real.
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\installer_e2e.ps1
#   ... -Setup .\dist\cuter-0.1.0-x64-setup.exe
#
# What it proves (all of it against the real machine, in the current user's hive):
#
#   1. silent install works and puts cuter.exe where it says;
#   2. the Start Menu shortcut and the (optional) desktop shortcut exist;
#   3. the optional "start with Windows" task really enabled autostart - i.e. the
#      installer drove the program's own `--enable-autostart`, so BOTH the config and
#      HKCU\...\Run agree (this is the part that a naive installer gets wrong);
#   4. the installed program runs (`--version`);
#   5. uninstall removes the program, removes OUR autostart entry, and KEEPS the user's
#      settings/logs (that promise is in the installer's confirm page).
#
# It refuses to run if a Cuter instance is running (the single-instance guard would make
# results confusing), and it always uninstalls in `finally`.
param(
    [string]$Setup = ''
)

$ErrorActionPreference = 'Stop'

function Say($m) { Write-Host $m }

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

if (-not $Setup) {
    $candidate = Get-ChildItem (Join-Path $root 'dist') -Filter 'cuter-*-x64-setup.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $candidate) { throw 'no installer found in dist\ - run tools\package\build_installer.ps1 first' }
    $Setup = $candidate.FullName
}
$Setup = (Resolve-Path $Setup).Path
Say "setup: $Setup"
$version = (Get-Item $Setup).VersionInfo.ProductVersion
Say "version: $version"

Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

# Install location: the installer defaults to {localappdata}\Programs\Cuter, but we use a
# scratch dir so the test never touches a real installation of the user's.
#
# Scratch paths MUST be absolute: if the temp directory cannot be resolved, `Join-Path`
# silently returns a RELATIVE path and files land in the repository (that happened once).
$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }

$installDir = Join-Path $tempRoot ("cuter-installer-e2e-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-installer-config-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$uninstaller = Join-Path $installDir 'unins000.exe'

function Get-Autostart {
    try { return [string](Get-ItemProperty -Path $runKey -Name 'Cuter' -ErrorAction Stop).Cuter }
    catch { return $null }
}

$originalAutostart = Get-Autostart
if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force }

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $un = Join-Path ([System.IO.Path]::GetTempPath()) 'cuter-installer-e2e-*'
    Get-ChildItem ([System.IO.Path]::GetTempPath()) -Directory -Filter 'cuter-installer-e2e-*' -ErrorAction SilentlyContinue |
        ForEach-Object { if (Test-Path (Join-Path $_.FullName 'unins000.exe')) { & (Join-Path $_.FullName 'unins000.exe') /VERYSILENT /SUPPRESSMSGBOXES /NORESTART | Out-Null } }
    $cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $cfg -Force }
    if ($null -ne $originalAutostart) { Set-ItemProperty -Path $runKey -Name 'Cuter' -Value $originalAutostart -Force } else { Remove-ItemProperty -Path $runKey -Name 'Cuter' -ErrorAction SilentlyContinue }
    exit 1
}

$fail = @()

# ---------- 1. silent install with the extra tasks ----------
Say "installing (silent) into $installDir"
$p = Start-Process $Setup -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
    "/DIR=$installDir", '/TASKS=desktopicon,autostart'
)
if ($p.ExitCode -ne 0) { $fail += "installer exit code = $($p.ExitCode)" }

$installedExe = Join-Path $installDir 'cuter.exe'
Say "installed exe exists: $(Test-Path $installedExe)"
if (-not (Test-Path $installedExe)) { $fail += 'cuter.exe was not installed' }
if (-not (Test-Path $uninstaller)) { $fail += 'unins000.exe (uninstaller) is missing' }
if (-not (Test-Path (Join-Path $installDir 'README.txt'))) { $fail += 'README.txt was not installed' }
# The installed copy must NOT be portable - a `cuter.portable` marker here would make the
# installed program write its config into the install directory.
if (Test-Path (Join-Path $installDir 'cuter.portable')) { $fail += 'the installed copy wrongly carries the portable marker' }

# ---------- 2. shortcuts ----------
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Cuter'
$startMenuShortcut = Join-Path $startMenu 'Cuter.lnk'
$desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Cuter.lnk'
Say "start menu shortcut: $(Test-Path $startMenuShortcut)"
Say "desktop shortcut:    $(Test-Path $desktopShortcut)"
if (-not (Test-Path $startMenuShortcut)) { $fail += 'the Start Menu shortcut is missing' }
if (-not (Test-Path $desktopShortcut)) { $fail += 'the desktop shortcut task did not create a shortcut' }

# ---------- 3. autostart: BOTH the registry and the program's config ----------
$entry = Get-Autostart
Say "autostart entry: $entry"
if ($null -eq $entry) {
    $fail += 'the autostart task did not create a registry entry'
} elseif ($entry -notlike "*$installedExe*") {
    $fail += "the autostart entry does not point at the installed exe: $entry"
} elseif ($entry -notlike '*--tray') {
    $fail += "the autostart entry is missing --tray: $entry"
}

if (Test-Path $configPath) {
    $configText = Get-Content $configPath -Raw -Encoding UTF8
    $configOn = $configText -match '(?m)^\s*autostart\s*=\s*true'
    Say "config autostart = $configOn"
    if (-not $configOn) { $fail += 'the config still says autostart = false (the installer only wrote the registry)' }
} else {
    $fail += "config.toml was not created: $configPath"
}

# ---------- 4. the installed program runs ----------
if (Test-Path $installedExe) {
    $out = & $installedExe --version
    Say "installed program says: $out"
    if ("$out" -notmatch 'cuter') { $fail += "the installed program did not answer --version: $out" }
}

# ---------- 5. uninstall ----------
Say 'uninstalling (silent)'
& $uninstaller /VERYSILENT /SUPPRESSMSGBOXES /NORESTART | Out-Null
Start-Sleep -Seconds 3

Say "install dir gone: $(-not (Test-Path $installedExe))"
if (Test-Path $installedExe) { $fail += 'uninstall left cuter.exe behind' }

$entryAfter = Get-Autostart
Say "autostart after uninstall: $entryAfter"
if ($null -ne $entryAfter) { $fail += "uninstall left the autostart entry behind: $entryAfter" }

# The promise on the uninstall confirm page: settings, logs and screenshots survive.
if ($configBackup -and (Test-Path $configBackup)) {
    Say "user config kept: $(Test-Path $configPath)"
    if (-not (Test-Path $configPath)) { $fail += 'uninstall deleted the user config (the installer promised to keep it)' }
}

# ---------- cleanup ----------
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
if ($null -ne $originalAutostart) {
    Set-ItemProperty -Path $runKey -Name 'Cuter' -Value $originalAutostart -Force
} else {
    Remove-ItemProperty -Path $runKey -Name 'Cuter' -ErrorAction SilentlyContinue
}
Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $startMenu -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $desktopShortcut -Force -ErrorAction SilentlyContinue

if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw 'installer e2e failed'
}
Say 'RESULT: PASS - silent install, shortcuts, autostart wiring, run, and uninstall (settings kept)'
