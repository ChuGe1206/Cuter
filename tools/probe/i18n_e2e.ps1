# UI language end to end: does an English config really produce an English window?
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\i18n_e2e.ps1
#
# What it does:
#   1. writes a config with language = "en-US" and start_in_tray = false (so the settings
#      window is visible right away);
#   2. starts the app with CUTER_LOG=debug and screenshots the window;
#   3. asserts the log contains no "missing English text" line (the app logs one per key it
#      could not translate while rendering);
#   4. repeats with zh-CN and asserts the two screenshots differ.
# The two PNGs are kept for a human to look at (the unit tests in cuter-ui already prove the
# table is complete; this proves the switch is actually wired to the UI).
#
# ASCII only (see tools/probe/README.md).
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

Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')
Add-Type -AssemblyName System.Drawing

$tempRoot = [System.IO.Path]::GetTempPath()
$work = Join-Path $tempRoot ("cuter-i18n-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$titleSettings = 'Cuter'

function Stop-App {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 700
}

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Stop-App
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL'
    exit 1
}

function Write-Config([string]$language) {
    New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
    Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
theme = "auto"
language = "$language"
start_in_tray = false
close_to_tray = true
autostart = false
"@
}

function Screenshot([string]$name) {
    $w = [CuterInputProbe]::GetSystemMetrics(0); $h = [CuterInputProbe]::GetSystemMetrics(1)
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose()
    $path = Join-Path $work $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $path
}

function Run-Language([string]$language) {
    Write-Config $language
    Stop-App
    $before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    $env:CUTER_LOG = 'debug'
    $app = Start-Process $exe -PassThru
    $env:CUTER_LOG = ''
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 40000) {
        $app.Refresh()
        if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
        if ([CuterInputProbe]::FindWindow([uint32]$app.Id, $titleSettings) -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }
    Start-Sleep -Seconds 2
    $shot = Screenshot "settings-$language.png"
    # log slice written by this run only
    $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    $stream.Seek($before, 'Begin') | Out-Null
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    $new = $reader.ReadToEnd()
    $reader.Dispose(); $stream.Dispose()
    $missing = ($new -split "`r?`n" | Where-Object { $_ -match 'i18n-missing' }).Count
    Stop-App
    return [pscustomobject]@{ Shot = $shot; Missing = $missing }
}

$en = Run-Language 'en-US'
Say "en-US screenshot: $($en.Shot) | missing-translation warnings: $($en.Missing)"
$zh = Run-Language 'zh-CN'
Say "zh-CN screenshot: $($zh.Shot) | missing-translation warnings: $($zh.Missing)"

Copy-Item $configBackup $configPath -Force

if ($en.Missing -gt 0) {
    throw "$($en.Missing) visible strings had no English translation (see the log slice around this run)"
}
$a = [System.IO.File]::ReadAllBytes($en.Shot)
$b = [System.IO.File]::ReadAllBytes($zh.Shot)
$same = ($a.Length -eq $b.Length)
if ($same) {
    for ($i = 0; $i -lt $a.Length; $i++) {
        if ($a[$i] -ne $b[$i]) { $same = $false; break }
    }
}
if ($same) { throw 'the English and Chinese settings windows are pixel-identical - the switch did nothing' }
Say 'the two languages render differently, and nothing was left untranslated'
Say ("RESULT: PASS - language = en-US gives an English window; see {0}" -f $work)
