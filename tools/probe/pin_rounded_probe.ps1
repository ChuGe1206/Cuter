# Rounded pin corners, checked in pixels.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\pin_rounded_probe.ps1
#
# The pin draws the image with its corners cut off (`pin.rounded_corners`). "Rounded" is a
# visual property, so the assertion is visual too, but it is still a *pixel* assertion:
#   1. put a SOLID red image on the clipboard, Alt+V to pin it;
#   2. screenshot the pin window and sample the four corner pixels and the centre;
#   3. with rounded_corners = true  -> each corner differs from the centre (the desktop shows
#      through the cut corner);
#      with rounded_corners = false -> each corner equals the centre (sharp corner).
# Step 3's negative half is what proves the switch is not decorative.
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

Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$tempRoot = [System.IO.Path]::GetTempPath()
$work = Join-Path $tempRoot ("cuter-pin-round-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force

$pinTitle = 'Cuter ' + [char]0x8D34 + [char]0x56FE     # window title fragment
$script:blocked = $false

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

function Write-Config([bool]$rounded) {
    $value = if ($rounded) { 'true' } else { 'false' }
    Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
theme = "auto"
language = "auto"
start_in_tray = true
close_to_tray = true
autostart = false
show_tray_notifications = false

[capture]
hide_own_windows = false
auto_detect_windows = false
snap_controls = false
show_magnifier = false
include_cursor = false
remember_last_region = false
play_sound = false

[pin]
opacity = 1.0
shadow = false
border = false
rounded_corners = $value
initial_scale = 1.0
groups = 1
active_group = 0

[history]
limit = 0

[ocr]
engine = "disabled"
"@
}

# A solid red 240x160 image on the clipboard: any non-red pixel inside the pin window is
# either the desktop showing through (rounded) or a border (both disabled here).
function Set-RedClipboard {
    $bmp = New-Object System.Drawing.Bitmap(240, 160)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(255, 200, 40, 40))
    $g.Dispose()
    [System.Windows.Forms.Clipboard]::SetImage($bmp)
    $bmp.Dispose()
}

function Run-Case([bool]$rounded, [string]$tag) {
    Write-Config $rounded
    Stop-App
    $app = Start-Process $exe -PassThru
    Start-Sleep -Seconds 4
    $app.Refresh()
    if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
    [void][CuterInputProbe]::FocusDesktop()
    Start-Sleep -Milliseconds 400

    Set-RedClipboard
    [CuterInputProbe]::MoveTo(700, 500) | Out-Null
    Start-Sleep -Milliseconds 300
    [CuterInputProbe]::AltKey(0x56)      # Alt+V = pin from clipboard

    $pin = [IntPtr]::Zero
    foreach ($i in 1..40) {
        Start-Sleep -Milliseconds 250
        $pin = [CuterInputProbe]::FindWindow([uint32]$app.Id, $pinTitle)
        if ($pin -ne [IntPtr]::Zero) { break }
    }
    if ($pin -eq [IntPtr]::Zero) { throw 'the pin window never appeared (Alt+V)' }
    Start-Sleep -Milliseconds 1200

    $rect = @(([CuterInputProbe]::Describe($pin)) -split '\|')
    $x = [int]$rect[1]; $y = [int]$rect[2]; $w = [int]$rect[3]; $h = [int]$rect[4]
    $shot = Join-Path $work "pin-$tag.png"
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose()
    $bmp.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)

    function Sample([int]$sx, [int]$sy) {
        $c = $bmp.GetPixel([Math]::Max(0, [Math]::Min($w - 1, $sx)), [Math]::Max(0, [Math]::Min($h - 1, $sy)))
        return "$($c.R),$($c.G),$($c.B)"
    }
    $centre = Sample ([int]($w / 2)) ([int]($h / 2))
    $corners = @(
        (Sample 1 1),
        (Sample ($w - 2) 1),
        (Sample 1 ($h - 2)),
        (Sample ($w - 2) ($h - 2))
    )
    $bmp.Dispose()
    Stop-App
    return [pscustomobject]@{
        Tag = $tag; Shot = $shot; Centre = $centre; Corners = $corners
        Different = @($corners | Where-Object { $_ -ne $centre }).Count
    }
}

$on = Run-Case $true 'rounded'
Say ("rounded_corners = true : centre {0}, corners {1} -> {2}/4 differ" -f $on.Centre, ($on.Corners -join ' '), $on.Different)
Say ("  screenshot: {0}" -f $on.Shot)
$off = Run-Case $false 'square'
Say ("rounded_corners = false: centre {0}, corners {1} -> {2}/4 differ" -f $off.Centre, ($off.Corners -join ' '), $off.Different)
Say ("  screenshot: {0}" -f $off.Shot)

Copy-Item $configBackup $configPath -Force

if ($on.Centre -ne '200,40,40') { throw "the pin did not render the red image (centre is $($on.Centre))" }
if ($on.Different -lt 4) { throw "with rounding on, only $($on.Different)/4 corners were cut" }
if ($off.Different -ne 0) { throw "with rounding off, $($off.Different)/4 corners were still cut - the switch does nothing" }
Say 'RESULT: PASS - corners are cut when rounded_corners is on, sharp when it is off'
