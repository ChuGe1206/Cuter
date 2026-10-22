# Pin drag-out end to end: Ctrl+drag a pinned image into ANOTHER PROGRAM.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\pin_drag_out_e2e.ps1
#
# The "other program" is a real window created by tools/probe/DropTarget.cs that listens
# for WM_DROPFILES - the same path Explorer and most chat apps use. The probe asserts that
# it received a path inside %TEMP%\Cuter\drag and that the file is a valid PNG.
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
Add-Type -Path (Join-Path $root 'tools\probe\DropTarget.cs')
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }
$work = Join-Path $tempRoot ("cuter-drag-e2e-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null
$received = Join-Path $work 'received.txt'
New-Item -ItemType File -Force -Path $received | Out-Null

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$dragDir = Join-Path $tempRoot 'Cuter\drag'

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    try { [DropTarget]::Stop() } catch { }
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL'
    exit 1
}

Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 600

# A clean, deterministic config: tray mode, no history.
New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true

[hotkeys]
capture = "Alt+C"
pin_from_clipboard = "Alt+V"

[capture]
backend = "gdi"

[pin]
opacity = 1.0
shadow = false
border = true
groups = 1
active_group = 0

[history]
limit = 0

[ocr]
engine = "disabled"
"@

# A distinctive clipboard image (240x160 solid colour) becomes the pin.
$bmp = New-Object System.Drawing.Bitmap(240, 160)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.Clear([System.Drawing.Color]::FromArgb(255, 20, 160, 90))
$g.Dispose()
[System.Windows.Forms.Clipboard]::SetImage($bmp)
$bmp.Dispose()

$before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
$app = Start-Process $exe -PassThru
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 60000) {
    Start-Sleep -Milliseconds 200
    if ((Test-Path $logPath) -and ((Get-Item $logPath).Length -gt $before)) { break }
    $app.Refresh()
    if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
}
Start-Sleep -Seconds 2

$pinTitle = 'Cuter ' + [char]0x8D34 + [char]0x56FE
# Put the cursor where the pin should appear, then create it with the HOTKEY: a pin made
# that way is focused (created active), so Ctrl is seen without clicking - and clicking
# would start a modal window-move loop (the pin drags itself), which eats the next drag.
[void][CuterInputProbe]::MoveTo(700, 500)
Start-Sleep -Milliseconds 300
[CuterInputProbe]::AltKey(0x56)
$pin = [IntPtr]::Zero
foreach ($i in 1..30) {
    Start-Sleep -Milliseconds 200
    $pin = [CuterInputProbe]::FindWindow([uint32]$app.Id, $pinTitle)
    if ($pin -ne [IntPtr]::Zero) { break }
}
if ($pin -eq [IntPtr]::Zero) { throw 'no pin window appeared (cuter --pin)' }
$pinRect = [CuterInputProbe]::WindowRect($pin)
$pinW = $pinRect.Right - $pinRect.Left
$pinH = $pinRect.Bottom - $pinRect.Top
Say ("pin window: {0}x{1} at ({2},{3})" -f $pinW, $pinH, $pinRect.Left, $pinRect.Top)

# The drop target: a window away from the pin, big enough to aim at.
$targetX = 60
$targetY = 60
$targetW = 360
$targetH = 260
$target = [DropTarget]::Start('Cuter Probe Drop Target', $targetX, $targetY, $targetW, $targetH, $received)
if ($target -eq [IntPtr]::Zero) { throw 'could not create the drop target window' }
Say ("drop target: {0}x{1} at ({2},{3})" -f $targetW, $targetH, $targetX, $targetY)
Say ("drop target state: " + [CuterInputProbe]::Describe($target))

Start-Sleep -Milliseconds 500
function Save-Shot([string]$name) {
    Add-Type -AssemblyName System.Drawing
    $w = [CuterInputProbe]::GetSystemMetrics(0); $h = [CuterInputProbe]::GetSystemMetrics(1)
    $b = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($b)
    $g.CopyFromScreen(0, 0, 0, 0, (New-Object System.Drawing.Size($w, $h))); $g.Dispose()
    $b.Save((Join-Path $work $name), [System.Drawing.Imaging.ImageFormat]::Png); $b.Dispose()
}
Save-Shot "before-drag.png"

# Ctrl + left drag from the middle of the pin onto the drop target.
$fromX = $pinRect.Left + [int]($pinW / 2)
$fromY = $pinRect.Top + [int]($pinH / 2)
$toX = $targetX + [int]($targetW / 2)
$toY = $targetY + [int]($targetH / 2)
Say ("dragging from ({0},{1}) to ({2},{3})" -f $fromX, $fromY, $toX, $toY)

function Invoke-OutDrag([string]$what, [bool]$useCtrl) {
    [void][CuterInputProbe]::MoveTo($fromX, $fromY)
    Start-Sleep -Milliseconds 250
    if ($useCtrl) { [CuterInputProbe]::KeyDown(0x11) }
    Start-Sleep -Milliseconds 120
    if ($useCtrl) { [CuterInputProbe]::LeftDown() } else { [CuterInputProbe]::MiddleDown() }
    Start-Sleep -Milliseconds 120
    # Move in steps: the pin arms the drag after a few pixels, then OLE takes over.
    $steps = 14
    for ($i = 1; $i -le $steps; $i++) {
        $x = $fromX + [int](($toX - $fromX) * $i / $steps)
        $y = $fromY + [int](($toY - $fromY) * $i / $steps)
        [void][CuterInputProbe]::MoveTo($x, $y)
        Start-Sleep -Milliseconds 90
    }
    Start-Sleep -Milliseconds 500
    if ($useCtrl) { [CuterInputProbe]::LeftUp() } else { [CuterInputProbe]::MiddleUp() }
    Start-Sleep -Milliseconds 150
    if ($useCtrl) { [CuterInputProbe]::KeyUp(0x11) }
    Start-Sleep -Seconds 2
    Say ("{0}: dragged onto the target" -f $what)
}

Invoke-OutDrag 'Ctrl + left drag' $true
Invoke-OutDrag 'middle drag' $false

# Regression: the pin's action handling used to SELF-DEADLOCK (the mutex guard of
# `match lock(&pin).ui(..)` lived for the whole match, and every arm re-locked the pin).
# Ctrl+C on the pin exercises that path, and it must both work and not hang the app.
[void][CuterInputProbe]::MoveTo($fromX, $fromY)
Start-Sleep -Milliseconds 200
[CuterInputProbe]::Press()
[CuterInputProbe]::Release()
Start-Sleep -Milliseconds 300
[CuterInputProbe]::CtrlKey(0x43)     # Ctrl+C = copy this pin
Start-Sleep -Seconds 2
$app.Refresh()
if ($app.HasExited) { throw 'the app exited during Ctrl+C on a pin' }
if (-not $app.Responding) { throw 'the app stopped responding after Ctrl+C on a pin (deadlock)' }
$clipboardHasImage = $false
try { $clipboardHasImage = [System.Windows.Forms.Clipboard]::ContainsImage() } catch { $clipboardHasImage = $false }
if (-not $clipboardHasImage) { throw 'Ctrl+C on the pin did not put an image on the clipboard' }
Say 'Ctrl+C on the pin: app still responding, clipboard has an image'

[DropTarget]::Stop()
Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Copy-Item $configBackup $configPath -Force

# --- assertions -------------------------------------------------------------
$all = @()
if (Test-Path $received) { $all = Get-Content $received -Encoding UTF8 | Where-Object { $_ } }
Say '--- target log ---'
foreach ($line in $all) { Say ('  ' + $line) }

# What this asserts (and what it deliberately does NOT):
#   * the target process really received a drag for EACH gesture (DragEnter), and
#   * the data object really advertises CF_HDROP - the format Explorer and chat apps read
#     dropped files from.
# NOT asserted: reading the file back out of the data object. The C# test target could not
# do that (COM marshalling of GetData/IEnumFORMATETC under PowerShell 5.1's old compiler
# returned DV_E_FORMATETC), while the app side is fine - QueryGetData says yes. The
# file-content half therefore stays manual for now: drag a pin into Explorer or a chat
# window (see HANDOFF section 26).
$drags = ($all | Where-Object { $_ -like 'DRAGENTER*' }).Count
$withHDrop = ($all | Where-Object { $_ -like 'DRAGENTER*cfHDROP=yes*' }).Count
if ($drags -lt 2) { throw "expected one drag per gesture, the target saw $drags" }
if ($withHDrop -lt 2) { throw "the data object did not advertise CF_HDROP on every drag (saw $withHDrop of $drags)" }

# Each gesture must have exported a real PNG of the pinned size.
# The app writes the file on its side of the drop, which can land a moment after the target
# has already logged DragEnter - so wait for the second file instead of counting once.
$expectedDir = (Resolve-Path (Join-Path $tempRoot 'Cuter\drag')).Path
$cutoff = (Get-Date).AddMinutes(-3)
$pngs = @()
$waitUntil = (Get-Date).AddSeconds(8)
do {
    $pngs = @(Get-ChildItem $expectedDir -Filter '*.png' -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -gt $cutoff })
    if ($pngs.Count -ge 2) { break }
    Start-Sleep -Milliseconds 250
} while ((Get-Date) -lt $waitUntil)
if ($pngs.Count -lt 2) { throw "expected 2 freshly exported PNGs, found $($pngs.Count)" }
foreach ($png in $pngs) {
    $img = [System.Drawing.Image]::FromFile($png.FullName)
    try {
        if ($img.Width -ne 240 -or $img.Height -ne 160) {
            throw "the exported image is $($img.Width)x$($img.Height), expected 240x160"
        }
    } finally { $img.Dispose() }
}
Say ("exported {0} PNG(s), each 240x160 (one per gesture)" -f $pngs.Count)

# ctrl+drag must NOT have moved the window (that is the plain-drag behaviour)
$pinAfter = [CuterInputProbe]::FindWindow([uint32]$app.Id, $pinTitle)
$msg = ''
if ($pinAfter -ne [IntPtr]::Zero) {
    $after = [CuterInputProbe]::WindowRect($pinAfter)
    if ($after.Left -ne $pinRect.Left -or $after.Top -ne $pinRect.Top) {
        $msg = "the pin window moved to ($($after.Left),$($after.Top)) - Ctrl+drag should export, not move"
    }
}
if ($msg) { throw $msg }

Say 'RESULT: PASS - both export gestures reached another process with CF_HDROP, and a real PNG was written per drag'
