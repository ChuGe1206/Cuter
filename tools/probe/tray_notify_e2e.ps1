# Tray notification (balloon) end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\tray_notify_e2e.ps1
#
# The app has no balloon API in `tray-icon`, so it registers a HIDDEN helper icon of its own
# (our root window as the host) and pushes NIF_INFO through it. This probe checks:
#   A. show_tray_notifications = true  -> the startup greeting is delivered (log says
#      delivered=True) and a screenshot of the notification area is kept for a human;
#   B. show_tray_notifications = false -> nothing is delivered at all.
# Judging "the config switch really works" needs B: a switch that only ever turns something on
# is exactly the "fake switch" this project refuses to ship.
#
# KNOWN ENVIRONMENT LIMIT: this development machine suppresses tray balloons completely - a
# control experiment with a VISIBLE icon (Shell_NotifyIconW returns added=True, shown=True)
# also shows nothing on screen, so the app cannot be blamed for the missing visual. That is
# why this probe asserts the API + the switch (delivered=True / no log line), and keeps the
# screenshots for a human on a normal machine.
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

Add-Type -AssemblyName System.Drawing
$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$tempRoot = [System.IO.Path]::GetTempPath()
$work = Join-Path $tempRoot ("cuter-tray-notify-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force

function Stop-App {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 700
}

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    Stop-App
    if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL'
    exit 1
}

# The greeting is text the app writes into the log; build the pattern from code points so this
# file stays pure ASCII (PowerShell 5.1 would read Chinese in a BOM-less script as GBK).
$greetTag = [string]([char]0x6258) + [char]0x76D8 + [char]0x901A + [char]0x77E5
function Run-Case([bool]$enabled, [string]$tag) {
    Stop-App
    Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
theme = "auto"
language = "auto"
start_in_tray = true
close_to_tray = true
autostart = false
show_tray_notifications = $($enabled.ToString().ToLower())

[capture]
hide_own_windows = true
auto_detect_windows = true
snap_controls = true
show_magnifier = false
include_cursor = false
remember_last_region = false
play_sound = false

[history]
limit = 0

[ocr]
engine = "disabled"
"@
    $before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    $app = Start-Process $exe -PassThru
    Start-Sleep -Milliseconds 1500
    $app.Refresh()
    if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
    # Screenshot right away: a balloon lives ~5-10 s, so this is the only reliable moment.
    $shot = Join-Path $work "notify-$tag.png"
    $w = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Width
    $h = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Height
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose(); $bmp.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()

    # 日志晚一点读：问候是在根窗口句柄就绪之后写的，可能比截图晚一两帧。
    Start-Sleep -Seconds 3
    $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    $stream.Seek($before, 'Begin') | Out-Null
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    $slice = $reader.ReadToEnd()
    $reader.Dispose(); $stream.Dispose()
    $greets = @($slice -split "`r?`n" | Where-Object { $_ -match ([regex]::Escape($greetTag)) })
    $delivered = @($greets | Where-Object { $_ -match 'True|true' }).Count
    Stop-App
    return [pscustomobject]@{ Shot = $shot; Lines = $greets.Count; Delivered = $delivered; Text = ($greets -join ' / ') }
}

Add-Type -AssemblyName System.Windows.Forms
$on = Run-Case $true 'on'
Say ("A. notifications ON  -> log lines {0}, delivered=True {1}" -f $on.Lines, $on.Delivered)
Say ("   screenshot: {0}" -f $on.Shot)
$off = Run-Case $false 'off'
Say ("B. notifications OFF -> log lines {0}, delivered=True {1}" -f $off.Lines, $off.Delivered)

Copy-Item $configBackup $configPath -Force
# Screenshots are kept (like the i18n / OCR probes) so a human can confirm the balloon.

if ($on.Lines -eq 0) { throw 'with notifications enabled the app never tried to notify (no log line)' }
if ($on.Delivered -eq 0) { throw 'the startup greeting was attempted but the shell refused it (delivered=False)' }
if ($off.Lines -ne 0) { throw "with notifications disabled the app still logged a notification ($($off.Lines) line(s))" }
Say ('RESULT: PASS - the greeting is delivered when enabled, and the switch really turns it off (screenshots in {0})' -f $work)
