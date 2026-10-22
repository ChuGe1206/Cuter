# Build the no-install (portable) zip and then PROVE it is portable.
#
# ASCII only (see tools/probe/README.md): every Chinese string that must appear in the
# generated files is built from code points, and the generated README is written as
# UTF-8 with BOM so Notepad shows it correctly.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\portable_zip.ps1
#   ... -Exe .\target\release\cuter.exe -OutDir .\dist
#
# What "portable" means in this project (see cuter-core/src/config.rs):
# an empty file named `cuter.portable` next to the exe switches the config directory,
# the log directory, the data directory and the default save directory to the exe's own
# folder. The verification step at the end runs the packaged copy from a scratch folder
# and checks that the log really landed next to the exe - and that the per-user
# %APPDATA% log was NOT touched.
param(
    [string]$Exe = '',
    [string]$OutDir = ''
)

$ErrorActionPreference = 'Stop'

function Say($m) { Write-Host $m }

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - run 'cargo build --release' first" }
$exe = (Resolve-Path $exe).Path
$dist = if ($OutDir) { $OutDir } else { Join-Path $root 'dist' }

# Version comes from the exe's own resource block: that is what an installer, the
# Explorer properties page and the auto-updater will all agree on.
$version = (Get-Item $exe).VersionInfo.FileVersion
if (-not $version) { throw "the exe has no version resource - did build.rs fail?" }
$product = (Get-Item $exe).VersionInfo.ProductName
Say "packaging $product $version from $exe"

$folderName = "cuter-$version-portable-x64"
$stage = Join-Path $dist $folderName
$zip = Join-Path $dist "$folderName.zip"

Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $zip -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item $exe (Join-Path $stage 'cuter.exe') -Force
# The marker is the whole point: without it this would be "a copied exe that still
# writes its config into %APPDATA%", i.e. not portable at all.
New-Item -ItemType File -Force -Path (Join-Path $stage 'cuter.portable') | Out-Null

# README (UTF-8 with BOM so Windows PowerShell/Notepad decode it correctly).
$readmePath = Join-Path $stage 'README.txt'
$readme = @"
$product $version - portable (Windows x64)
===============================================

How to use
  1. Unzip anywhere (a USB stick works too) - keep the folder together.
  2. Run cuter.exe. It stays in the notification area; press Alt+C to capture.
  3. No installer, no registry writes except the optional "start with Windows"
     switch in Settings - General.

Why it is portable
  The empty file "cuter.portable" next to cuter.exe tells the program to keep
  everything in this folder:

    config.toml        your settings
    logs\cuter.log     log file (rotated, 3 files, 2 MB each)
    Screenshots\       default folder for saved screenshots
    data\              cache / models

  Delete that file and it goes back to %APPDATA%\Cuter.

CLI
  cuter.exe --full --out shot.png     capture the whole desktop, no UI
  cuter.exe --monitor --copy          capture the monitor under the cursor
  cuter.exe --region 0,0,800,600      capture a region
  cuter.exe --list-monitors           print monitor geometry and scaling
  cuter.exe --help

Upgrade: replace cuter.exe with the new one. Your config.toml and screenshots stay.
         (Set "autostart = true" again after moving the folder.)
"@
Set-Content -Path $readmePath -Value $readme -Encoding UTF8

Say "staged into $stage"
Get-ChildItem $stage | ForEach-Object { Say ("  {0,-16} {1,10} bytes" -f $_.Name, $_.Length) }

Compress-Archive -Path $stage -DestinationPath $zip -Force
$zipItem = Get-Item $zip
Say ("zip: {0} ({1:N0} bytes)" -f $zipItem.FullName, $zipItem.Length)

# ---------------------------------------------------------------- verification
# Extract to a scratch folder and run the packaged copy. Two assertions:
#   1. logs\cuter.log appears NEXT TO THE EXE (portable mode is detected);
#   2. the per-user %APPDATA%\Cuter\logs\cuter.log is untouched.
$verify = Join-Path $dist 'verify-portable'
Remove-Item $verify -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $verify | Out-Null
Expand-Archive -Path $zip -DestinationPath $verify -Force

$inner = Join-Path $verify $folderName
$packagedExe = Join-Path $inner 'cuter.exe'
if (-not (Test-Path $packagedExe)) { throw "the zip does not contain $folderName\cuter.exe" }
if (-not (Test-Path (Join-Path $inner 'cuter.portable'))) { throw 'the zip is missing the portable marker' }

$appDataLog = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$before = if (Test-Path $appDataLog) { (Get-Item $appDataLog) } else { $null }

& $packagedExe --list-monitors | Out-Null

$portableLog = Join-Path $inner 'logs\cuter.log'
Say "portable log: $portableLog exists=$(Test-Path $portableLog)"
if (-not (Test-Path $portableLog)) { throw 'portable mode did not take effect: no logs\cuter.log next to the exe' }

$after = if (Test-Path $appDataLog) { (Get-Item $appDataLog) } else { $null }
$untouched = $true
if ($before -and $after) {
    $untouched = ($before.Length -eq $after.Length) -and ($before.LastWriteTimeUtc -eq $after.LastWriteTimeUtc)
}
Say "per-user log untouched: $untouched"
if (-not $untouched) { throw 'the portable copy wrote into %APPDATA% - it is not portable' }

Say ''
Say "RESULT: PASS - $folderName.zip ($([math]::Round($zipItem.Length / 1MB, 2)) MB) is portable and self-contained"
Say "        (extracted copy kept at $verify for inspection)"
