# OCR panel + text<->position linkage, end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\ocr_linkage_e2e.ps1
#
# The flow is the one a user does: open a window with known text, Alt+C, drag over the text,
# Ctrl+O (= OCR this selection), then the result panel appears. The probe:
#   1. asserts the panel window shows up;
#   2. presses L to switch to the line list ("逐行对照");
#   3. screenshots the panel, then walks the cursor down the line column, screenshotting until
#      one row's hover really repaints the panel (that is the text -> image half of the linkage);
#   4. keeps the two PNGs next to the probe's temp dir so a human can confirm the highlight
#      lands on the right box (the app's own unit test covers the hit test geometry).
#
# The target is tools/probe/OcrTarget.cs - three 70px monospace ASCII lines. A small mixed-script
# Notepad file used to be the target, and the engine sometimes returned two lines or garbage,
# which made the hover rows land on nothing (see docs/HANDOFF.md 69.6).
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
Add-Type -Path (Join-Path $root 'tools\probe\OcrTarget.cs')
Add-Type -AssemblyName System.Drawing
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }
$work = Join-Path $tempRoot ("cuter-ocr-link-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$titleShot = 'Cuter ' + [char]0x622A + [char]0x56FE
$titleOcr = 'Cuter ' + [char]0x8BC6 + [char]0x522B + [char]0x7ED3 + [char]0x679C
$script:blocked = $false

function Blocked-Message([string]$text) {
    $fg = [CuterInputProbe]::ForegroundProcess()
    if ($fg -and $fg -notmatch '^(cuter|powershell|pwsh|cmd|conhost|explorer|notepad|WindowsTerminal)$') {
        $script:blocked = $true
        Say "foreground app is '$fg' - [$([CuterInputProbe]::ForegroundTitle())]"
        return "SKIPPED_KEYBOARD_GRABBED: $text"
    }
    return $text
}

function Stop-Everything {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    try { [OcrTarget]::Stop() } catch { }
}

trap {
    if ($script:blocked -or $_.Exception.Message -like 'SKIPPED_KEYBOARD_GRABBED*') {
        Write-Host "SKIPPED: $($_.Exception.Message -replace '^SKIPPED_KEYBOARD_GRABBED: ', '')"
    } else {
        Write-Host "FAILED: $($_.Exception.Message)"
    }
    Stop-Everything
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $configPath -Force }
    if ($script:blocked) {
        Write-Host 'RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive'
        exit 2
    }
    Write-Host 'RESULT: FAIL'
    exit 1
}

Stop-Everything
Start-Sleep -Milliseconds 600

# Target: three big monospace ASCII lines (tools/probe/OcrTarget.cs).
#
# This used to be Notepad with a small mixed-script file, and that was the flake: the engine
# sometimes returned two lines instead of three, sometimes garbage, and the hover rows below
# then landed on nothing. Big ASCII glyphs give one box per line with an even height, and the
# line rects come from the target itself instead of being guessed.
$targetX = 360
$targetY = 260
$target = [OcrTarget]::Start($targetX, $targetY)
if ($target -eq [IntPtr]::Zero) { throw 'the target window never appeared' }
Start-Sleep -Milliseconds 600
[void][CuterInputProbe]::Activate($target)
Start-Sleep -Milliseconds 400
Say "target: $([CuterInputProbe]::Describe($target))"

$lineRects = @()
foreach ($i in 0, 1, 2) {
    $r = @(([OcrTarget]::LineRect($i)) -split '\|')
    if ($r.Count -ne 4) { throw "the target did not report the rect of line $i" }
    $lineRects += , @([int]$r[0], [int]$r[1], [int]$r[2], [int]$r[3])
}
# Generous margins: the selection must contain the whole glyphs plus some white space, otherwise
# the preview looks clipped and the archived evidence shot is ambiguous about whether the *panel*
# crops the image or the *probe* framed it too tight.
$selLeft = ($lineRects | ForEach-Object { $_[0] } | Measure-Object -Minimum).Minimum - 40
$selTop = ($lineRects | ForEach-Object { $_[1] } | Measure-Object -Minimum).Minimum - 30
$selRight = ($lineRects | ForEach-Object { ($_[0] + $_[2]) } | Measure-Object -Maximum).Maximum + 40
$selBottom = ($lineRects | ForEach-Object { ($_[1] + $_[3]) } | Measure-Object -Maximum).Maximum + 30
Say ("selection: {0},{1} {2}x{3}" -f $selLeft, $selTop, ($selRight - $selLeft), ($selBottom - $selTop))

New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true

[hotkeys]
capture = "Alt+C"

[capture]
hide_own_windows = true
auto_detect_windows = false
snap_controls = false
show_magnifier = false
include_cursor = false
remember_last_region = false
play_sound = false

[ocr]
engine = "builtin"
paragraph = false
auto_after_capture = false

[history]
limit = 0
"@

$before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
$app = Start-Process $exe -PassThru
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt 60000) {
    if ((Test-Path $logPath) -and ((Get-Item $logPath).Length -gt $before)) { break }
    $app.Refresh()
    if ($app.HasExited) { throw "the app exited immediately ($($app.ExitCode))" }
    Start-Sleep -Milliseconds 200
}
Start-Sleep -Seconds 2
[void][CuterInputProbe]::FocusDesktop()
Start-Sleep -Milliseconds 400

# ---------- capture + OCR ----------
$overlay = [IntPtr]::Zero
foreach ($attempt in 1..4) {
    [CuterInputProbe]::AltKey(0x43)
    foreach ($i in 1..25) {
        Start-Sleep -Milliseconds 200
        $overlay = [CuterInputProbe]::FindWindow([uint32]$app.Id, $titleShot)
        if ($overlay -ne [IntPtr]::Zero) { break }
    }
    if ($overlay -ne [IntPtr]::Zero) { break }
    [CuterInputProbe]::Key(0x1B)
    [void][CuterInputProbe]::FocusDesktop()
    Start-Sleep -Milliseconds 700
}
if ($overlay -eq [IntPtr]::Zero) { throw (Blocked-Message 'the overlay never appeared after Alt+C') }
Start-Sleep -Milliseconds 600

# Drag over the three lines.
[CuterInputProbe]::Drag($selLeft, $selTop, $selRight, $selBottom)
Start-Sleep -Milliseconds 500
[CuterInputProbe]::CtrlKey(0x4F)   # Ctrl+O = OCR this selection
Say 'asked the overlay for OCR (Ctrl+O)'

$panel = [IntPtr]::Zero
foreach ($i in 1..60) {
    Start-Sleep -Milliseconds 250
    $panel = [CuterInputProbe]::FindWindow([uint32]$app.Id, $titleOcr)
    if ($panel -ne [IntPtr]::Zero) { break }
}
if ($panel -eq [IntPtr]::Zero) { throw (Blocked-Message 'the OCR panel never appeared (Ctrl+O)') }
[void][CuterInputProbe]::Activate($panel)
Start-Sleep -Seconds 2
Say "ocr panel: $([CuterInputProbe]::Describe($panel))"

$pRect = @(([CuterInputProbe]::Describe($panel)) -split '\|')
$pL = [int]$pRect[1]; $pT = [int]$pRect[2]; $pW = [int]$pRect[3]; $pH = [int]$pRect[4]

# Only the image pane (the left part of the panel) is captured. Comparing the whole panel would
# also accept a repaint somewhere else (a button lighting up, the elapsed-ms text ticking); the
# assertion is that hovering a *list row* repaints the *image*, which is the linkage itself.
function Save-Panel([string]$name) {
    $bmp = New-Object System.Drawing.Bitmap($imgW, $imgH)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($imgX, $imgY, 0, 0, (New-Object System.Drawing.Size($imgW, $imgH)))
    $g.Dispose()
    $path = Join-Path $work $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $path
}

# The whole panel, for the archived evidence shot (the assertion above only looks at the image
# pane, but a reader wants to see the row list next to the highlighted box in one frame).
function Save-FullPanel([string]$name) {
    $bmp = New-Object System.Drawing.Bitmap($pW, $pH)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($pL, $pT, 0, 0, (New-Object System.Drawing.Size($pW, $pH)))
    $g.Dispose()
    $path = Join-Path $work $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $path
}

function Test-SameFile([string]$a, [string]$b) {
    $x = [System.IO.File]::ReadAllBytes($a)
    $y = [System.IO.File]::ReadAllBytes($b)
    if ($x.Length -ne $y.Length) { return $false }
    for ($i = 0; $i -lt $x.Length; $i++) { if ($x[$i] -ne $y[$i]) { return $false } }
    return $true
}

# ---------- line list + hover ----------
[CuterInputProbe]::Key(0x4C)   # L = switch to the line list
Start-Sleep -Milliseconds 900

# The rows live in the right half of the panel; the boxes live in the left pane.
$rowX = $pL + [int]($pW * 0.72)
$imgX = $pL + [int]($pW * 0.01)
$imgY = $pT + [int]($pH * 0.09)
$imgW = [int]($pW * 0.44)
$imgH = [int]($pH * 0.40)

$idle = Save-Panel 'panel-idle.png'

# Row heights follow the theme font and how many lines the engine found, so instead of guessing
# one y the probe walks down the column and takes the first row whose hover repaints the image
# pane. The assertion is "hovering a row paints its box", not "row 2 sits at +120".
$painted = $false
$hovered = $idle
for ($dy = 96; $dy -le ($pH - 48); $dy += 16) {
    [void][CuterInputProbe]::MoveTo($rowX, ($pT + $dy))
    Start-Sleep -Milliseconds 220
    $shot = Save-Panel 'panel-hover.png'
    if (-not (Test-SameFile $idle $shot)) {
        $painted = $true
        $hovered = $shot
        Say "hovering a row repainted the image pane at y=$($pT + $dy)"
        [void](Save-FullPanel 'panel-hover-full.png')
        break
    }
}
Say "screenshots: $idle / $hovered"

Stop-Everything
Copy-Item $configBackup $configPath -Force

# ---------- assertions ----------
if (-not $painted) { throw 'no hovered row repainted the image pane - the text -> image highlight is missing' }
Say 'hovering a row repainted the image pane (text -> image highlight is alive)'
Say ("RESULT: PASS - OCR panel opened, line list works, hovering a line highlights its box (see {0})" -f $work)
