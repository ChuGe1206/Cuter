# Fetch the packaging toolchain (Inno Setup 6 + the official Chinese Simplified messages).
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\package\get_innosetup.ps1
#
# Why a script instead of "install it yourself":
#   * the compiler must be reproducible for whoever builds the installer next;
#   * it installs PORTABLE into %LOCALAPPDATA%\Cuter\tools\innosetup, so nothing is added
#     to Program Files / Start Menu / the uninstall list of the machine;
#   * the download is verified (Authenticode signature) before it is run.
#
# `build_installer.ps1` calls this automatically when ISCC.exe is missing.
param(
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

function Say($m) { Write-Host $m }

$toolDir = Join-Path $env:LOCALAPPDATA 'Cuter\tools\innosetup'
$iscc = Join-Path $toolDir 'ISCC.exe'
$languageFile = Join-Path $toolDir 'Languages\ChineseSimplified.isl'

if ((Test-Path $iscc) -and (Test-Path $languageFile) -and -not $Force) {
    Say "Inno Setup already present: $iscc"
    exit 0
}

$tmp = Join-Path $env:TEMP 'cuter-innosetup'
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$installer = Join-Path $tmp 'innosetup-6.7.3.exe'
$url = 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe'

if (-not (Test-Path $installer)) {
    Say "downloading $url"
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $url -OutFile $installer -UseBasicParsing
}

# Refuse to run anything whose signature is not valid: this is a third-party binary
# that we are about to execute on the machine.
$sig = Get-AuthenticodeSignature $installer
Say ("signature: {0} ({1})" -f $sig.Status, $sig.SignerCertificate.Subject)
if ($sig.Status -ne 'Valid') { throw "the Inno Setup download is not validly signed ($($sig.Status))" }

Say "installing (portable) into $toolDir"
$p = Start-Process $installer -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/PORTABLE=1', "/DIR=$toolDir"
)
if ($p.ExitCode -ne 0) { throw "Inno Setup installer exited with $($p.ExitCode)" }
if (-not (Test-Path $iscc)) { throw "ISCC.exe not found after install: $iscc" }

# The Chinese Simplified translation is official but lives in the repo's Languages
# folder (not in the installed set), so pull it explicitly.
if (-not (Test-Path $languageFile)) {
    $languageUrl = 'https://raw.githubusercontent.com/jrsoftware/issrc/refs/heads/main/Files/Languages/ChineseSimplified.isl'
    Say "fetching Chinese Simplified messages"
    Invoke-WebRequest -Uri $languageUrl -OutFile $languageFile -UseBasicParsing
}

Say "ready: $iscc"
& $iscc '/?'
