# Build the Windows installer (cuter-<version>-x64-setup.exe) with Inno Setup 6.
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\build_installer.ps1
#   ... -Exe .\target\release\cuter.exe -OutDir .\dist
#
# It fetches the compiler if needed (get_innosetup.ps1), stages the payload, and runs
# ISCC. The version in the installer's file name and in the Programs-and-Features entry
# is read from the exe's own version resource, so it can never drift from the product.
#
# Verification lives in a separate probe (tools\probe\installer_e2e.ps1) because that one
# actually installs and uninstalls - do not mix "build" with "modify the machine".
param(
    [string]$Exe = '',
    [string]$OutDir = '',
    [switch]$SkipToolchainCheck
)

$ErrorActionPreference = 'Stop'

function Say($m) { Write-Host $m }

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - run 'cargo build --release' first" }
$exe = (Resolve-Path $exe).Path

$dist = if ($OutDir) { $OutDir } else { Join-Path $root 'dist' }
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$dist = (Resolve-Path $dist).Path

$iscc = Join-Path $env:LOCALAPPDATA 'Cuter\tools\innosetup\ISCC.exe'
$languageFile = Join-Path $env:LOCALAPPDATA 'Cuter\tools\innosetup\Languages\ChineseSimplified.isl'
if (-not (Test-Path $iscc) -or -not (Test-Path $languageFile)) {
    if ($SkipToolchainCheck) { throw "ISCC.exe / ChineseSimplified.isl missing; run tools\package\get_innosetup.ps1" }
    Say 'Inno Setup not found - fetching it (portable, into %LOCALAPPDATA%\Cuter\tools)'
    & (Join-Path $PSScriptRoot 'get_innosetup.ps1')
    if (-not (Test-Path $iscc)) { throw "still missing: $iscc" }
}
if (-not (Test-Path $languageFile)) { throw "missing the Chinese messages file: $languageFile" }

# ---- payload: the exe plus an installed-flavour README (no `cuter.portable` marker:
#      an installed copy must use %APPDATA%, that is what the marker would switch off).
$payload = Join-Path $dist 'installer-payload'
Remove-Item $payload -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $payload | Out-Null
Copy-Item $exe (Join-Path $payload 'cuter.exe') -Force

$version = (Get-Item $exe).VersionInfo.FileVersion
if (-not $version) { throw 'the exe has no version resource - check crates/cuter-app/build.rs' }

$readme = @"
Cuter $version (Windows x64, installed)
===============================================

How to use
  Press Alt+C in any program: the screen freezes, drag a region, annotate, then
  Enter to copy / Ctrl+S to save / Ctrl+T to pin it on screen.
  The program stays in the notification area; right-click the tray icon for the menu.

Where things live
  program      %LOCALAPPDATA%\Programs\Cuter
  settings     %APPDATA%\Cuter\config.toml
  logs         %APPDATA%\Cuter\logs\cuter.log   (rotated: 3 files, 2 MB each)
  screenshots  %USERPROFILE%\Pictures\Cuter     (changeable in Settings)

Uninstalling keeps your settings, logs and screenshots.
  Settings - General has a "start with Windows" switch; the installer can set it too.
  Use "Settings - Diagnostics" to open the config or log folder.

Prefer a no-install copy? Use the portable zip: unzip anywhere, run cuter.exe.
It keeps everything (config, logs, screenshots) in its own folder.
"@
Set-Content -Path (Join-Path $payload 'README.txt') -Value $readme -Encoding UTF8

Say "payload: $payload (cuter.exe $((Get-Item (Join-Path $payload 'cuter.exe')).Length) bytes)"
Say "compiling with $iscc"

& $iscc "/DPayloadDir=$payload" "/DOutputDir=$dist" (Join-Path $PSScriptRoot 'cuter.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }

$setup = Join-Path $dist "cuter-$version-x64-setup.exe"
if (-not (Test-Path $setup)) { throw "expected output not found: $setup" }
$setupItem = Get-Item $setup
$info = $setupItem.VersionInfo

Say ''
Say ("setup: {0} ({1:N2} MB)" -f $setupItem.FullName, ($setupItem.Length / 1MB))
Say ("       FileVersion={0} ProductName={1}" -f $info.FileVersion, $info.ProductName)
Say ("RESULT: built cuter-$version-x64-setup.exe")
Say '        (install it for real with: tools\probe\installer_e2e.ps1)'
