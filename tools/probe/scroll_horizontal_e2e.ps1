# Horizontal scroll capture end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\scroll_horizontal_e2e.ps1
#
# The target is a window (tools/probe/ScrollTarget.cs) whose content is 4000 px wide and
# scrolls horizontally on WM_MOUSEHWHEEL. A real user flow:
#   1. Alt+S enters scroll-capture mode;
#   2. drag a selection inside the target, Enter starts collecting;
#   3. send real horizontal wheel events (the direction is auto-detected by the app);
#   4. activate the control window, Enter finishes;
#   5. assert the saved long image is WIDER than the selection and exactly as tall.
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
Add-Type -Path (Join-Path $root 'tools\probe\ScrollTarget.cs')
Add-Type -AssemblyName System.Drawing
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }
$work = Join-Path $tempRoot ("cuter-hscroll-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$saveDir = Join-Path $work 'shots'
New-Item -ItemType Directory -Force -Path $saveDir | Out-Null

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $work 'config-backup.toml'
if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force }
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$titleShot = 'Cuter ' + [char]0x622A + [char]0x56FE
$titleScroll = 'Cuter ' + [char]0x6EDA + [char]0x52A8 + [char]0x622A + [char]0x5C4F
$script:blocked = $false

function Stop-Everything {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    try { [ScrollTarget]::Stop() } catch { }
    Stop-Everything
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL'
    exit 1
}

# A scroll session needs the target to be visible while collecting, and the probe usually
# runs over a maximised browser - so the target goes topmost.
Stop-Everything
Start-Sleep -Milliseconds 600

$targetX = 320
$targetY = 260
$targetW = 900
$targetH = 420
$target = [ScrollTarget]::Start('Cuter Probe Scroll Target', $targetX, $targetY, $targetW, $targetH)
if ($target -eq [IntPtr]::Zero) { throw 'could not create the scroll target' }
Start-Sleep -Milliseconds 800
[void][CuterInputProbe]::Activate($target)
Start-Sleep -Milliseconds 400
$tRect = @(([CuterInputProbe]::Describe($target)) -split '\|')
Say "scroll target: $($tRect[1]),$($tRect[2]) ${targetW}x${targetH}"

New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true

[hotkeys]
capture = "Alt+C"
scroll_capture = "Alt+S"

[capture]
hide_own_windows = true
auto_detect_windows = false
snap_controls = false
show_magnifier = false
include_cursor = false
remember_last_region = false
play_sound = false

[scroll]
interval_ms = 300
max_height = 20000

[save]
dir = '$saveDir'
format = "png"
auto_save = true
ask_path_on_save = false
auto_copy = false
reveal_after_save = false

[history]
limit = 0

[ocr]
engine = "disabled"
"@

$before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
$cuter = Start-Process $exe -PassThru
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 60000) {
    if ((Test-Path $logPath) -and ((Get-Item $logPath).Length -gt $before)) { break }
    $cuter.Refresh()
    if ($cuter.HasExited) { throw "the app exited immediately ($($cuter.ExitCode))" }
    Start-Sleep -Milliseconds 200
}
Start-Sleep -Seconds 2

# ---------- 1. Alt+S and make a selection ----------
[void][CuterInputProbe]::Activate($target)
Start-Sleep -Milliseconds 300
$overlay = [IntPtr]::Zero
foreach ($attempt in 1..4) {
    [CuterInputProbe]::AltKey(0x53)
    foreach ($i in 1..25) {
        Start-Sleep -Milliseconds 200
        $overlay = [CuterInputProbe]::FindWindow([uint32]$cuter.Id, $titleShot)
        if ($overlay -ne [IntPtr]::Zero) { break }
    }
    if ($overlay -ne [IntPtr]::Zero) { break }
    [CuterInputProbe]::Key(0x1B)
    [void][CuterInputProbe]::FocusDesktop()
    Start-Sleep -Milliseconds 700
}
if ($overlay -eq [IntPtr]::Zero) { throw 'the overlay never appeared after Alt+S' }
Start-Sleep -Milliseconds 600

# A selection well inside the target's client area (away from the border/title bar).
$selLeft = $targetX + 60
$selTop = $targetY + 70
$selRight = $selLeft + 700
$selBottom = $selTop + 320
$selW = $selRight - $selLeft
$selH = $selBottom - $selTop
Say "selection: ($selLeft,$selTop)-($selRight,$selBottom) = ${selW}x${selH}"

[CuterInputProbe]::Drag($selLeft, $selTop, $selRight, $selBottom)
Start-Sleep -Milliseconds 400
[CuterInputProbe]::Key(0x0D)   # Enter: start collecting
Start-Sleep -Seconds 2

$panel = [CuterInputProbe]::FindWindow([uint32]$cuter.Id, $titleScroll)
if ($panel -eq [IntPtr]::Zero) { throw 'the scroll control window did not appear' }
Say "scroll panel: $([CuterInputProbe]::Describe($panel))"

# The overlay reports the region it actually took; the synthetic drag has raced twice today
# (a 700x320 drag came out 642x294 because the press anchor lagged behind the first move).
# Reading it back is the only way to tell "the product stitched wrong" from "the probe dragged wrong".
$regionLine = @(Get-Content $logPath -Encoding UTF8 -Tail 60 -ErrorAction SilentlyContinue |
    Select-String -Pattern 'Rect \{ x: (\d+), y: (\d+), w: (\d+), h: (\d+) \}' | Select-Object -Last 1)
if ($regionLine.Count -gt 0) {
    $null = $regionLine[0].Line -match 'Rect \{ x: (\d+), y: (\d+), w: (\d+), h: (\d+) \}'
    $actW = [int]$Matches[3]
    $actH = [int]$Matches[4]
    Say "actual region: $($Matches[1]),$($Matches[2]) ${actW}x${actH}"
    if ($actW -ne $selW -or $actH -ne $selH) {
        throw "the overlay picked a ${actW}x${actH} region - the drag did not start and end where the probe asked (${selW}x${selH} at $selLeft,$selTop)"
    }
}

# ---------- 2. scroll the target horizontally ----------
[void][CuterInputProbe]::Activate($target)
[void][CuterInputProbe]::MoveTo(($selLeft + [int]($selW / 2)), ($selTop + [int]($selH / 2)))
Start-Sleep -Milliseconds 300
# Informational only: the native EDIT control consumes the wheel itself, so our own counter
# may stay at 0 even when the content really scrolled. The ground truth is the control's own
# scroll position (`ScrollPos`), and the real assertion is the stitched width below.
$beforeScroll = [ScrollTarget]::ScrollX()
$beforePos = [ScrollTarget]::ScrollPos()
foreach ($step in 1..10) {
    [CuterInputProbe]::HWheel(1)
    Start-Sleep -Milliseconds 350
}
$afterPos = [ScrollTarget]::ScrollPos()
Say "target scroll counter: $beforeScroll -> $([ScrollTarget]::ScrollX()) px (informational)"
Say "edit scroll pos: $beforePos -> $afterPos"
$moved = ([int](($afterPos -split '\|')[0])) -gt ([int](($beforePos -split '\|')[0]))
if (-not $moved) {
    # Seen on real hardware: the same script sometimes does not move this native EDIT at all
    # (ten synthetic `WM_MOUSEHWHEEL` events in a row and the scroll position stays at 0) - the
    # window never becomes the wheel target. Rather than failing the whole run on that, let the
    # target scroll itself through the same `EM_LINESCROLL` path: the pixels on screen really
    # move, which is all the stitcher cares about.
    Say 'the synthetic wheel did not move the target - falling back to a self-scrolling target'
    [ScrollTarget]::StartAutoScroll(300, 120)
    Start-Sleep -Seconds 4
    [ScrollTarget]::StopAutoScroll()
    Say "edit scroll pos after the fallback: $([ScrollTarget]::ScrollPos())"
}
Start-Sleep -Seconds 2

# ---------- 3. finish ----------
[void][CuterInputProbe]::Activate($panel)
Start-Sleep -Milliseconds 300
[CuterInputProbe]::Key(0x0D)   # Enter: finish
Start-Sleep -Seconds 3

Stop-Everything
try { [ScrollTarget]::Stop() } catch { }
Copy-Item $configBackup $configPath -Force

# ---------- 4. assertions ----------
$files = @(Get-ChildItem $saveDir -Filter *.png -ErrorAction SilentlyContinue)
if ($files.Count -eq 0) { throw 'nothing was saved (the scroll session did not deliver)' }
$newest = $files | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$img = [System.Drawing.Image]::FromFile($newest.FullName)
try {
    Say ("saved long image: {0}x{1}" -f $img.Width, $img.Height)
    if ($img.Height -ne $selH) {
        throw "the stitched image is $($img.Height) px tall, expected the selection height $selH"
    }
    if ($img.Width -le $selW) {
        throw "the stitched image is $($img.Width) px wide - horizontal scrolling was not stitched (selection was $selW)"
    }
} finally { $img.Dispose() }

Say 'RESULT: PASS - horizontal scrolling was detected and stitched into a wider long image'
