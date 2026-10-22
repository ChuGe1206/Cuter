# hide_own_windows end-to-end test: our own windows must never end up in the shot.
#
# ASCII only (see tools/probe/README.md) - that is why every Chinese string this
# script needs (the overlay title, the app's stage labels) is built from code points
# at runtime instead of being typed into the file.
#
# The scenario is exactly what a user does:
#   1. a Notepad window is parked at a fixed place - it is the static background
#      whose pixels we can predict;
#   2. reference capture of the Notepad CLIENT area (call it R): plain `--region`,
#      nothing of ours on screen;
#   3. the tray instance is asked to show its settings window, which is then moved
#      to cover R exactly;
#   4. capture R again -> the deviation against the reference MUST be large. That is
#      the "teeth" check: without it this test would also pass if the region simply
#      did not contain our window at all;
#   5. Alt+C (settings window still open) -> drag exactly R -> Enter (auto save);
#   6. the saved PNG must match the reference within noise, i.e. the settings window
#      that was sitting on R is gone from the frozen frame.
#
# Step 6 is the assertion that matters. With the "wait for our own window to
# disappear" step broken, the frozen frame still contains the settings window and
# the deviation jumps from ~1 to tens.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\hide_own_windows_e2e.ps1
#   ... -Exe .\target\release\cuter.exe
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

# Scratch paths MUST be absolute: if the temp directory cannot be resolved, `Join-Path`
# silently returns a RELATIVE path and files land in the repository (that happened once).
$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-hide-config-backup-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$outDir = Join-Path $root 'target\e2e\hide'
$logPath = Join-Path $outDir 'cuter-stderr.log'
$noteFile = Join-Path $tempRoot 'cuter-hide-target.txt'

# Any failure must leave the machine clean: a leftover instance keeps the global
# hotkey, and a half-written config would silently change the user's settings.
# Paths are rebuilt here because the variables above may not exist yet when the
# failure happens early.
trap {
    if ($script:blocked -or $_.Exception.Message -like 'SKIPPED_KEYBOARD_GRABBED*') {
        Write-Host "SKIPPED: $($_.Exception.Message -replace '^SKIPPED_KEYBOARD_GRABBED: ', '')"
    } else {
        Write-Host "FAILED: $($_.Exception.Message)"
    }
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $cfg -Force }
    if ($script:blocked) {
        Write-Host 'RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive'
        exit 2
    }
    exit 1
}

# A VM window or a remote-desktop client grabs the keyboard; synthesized Alt+C / Enter
# then reach nobody. That looks like an app bug but is not - report SKIPPED (exit 2).
$script:blocked = $false

function Blocked-Message([string]$text) {
    $fg = [CuterInputProbe]::ForegroundProcess()
    $title = [CuterInputProbe]::ForegroundTitle()
    if ($fg -and $fg -notmatch '^(cuter|powershell|pwsh|cmd|conhost|explorer|WindowsTerminal)$') {
        $script:blocked = $true
        Say "foreground app is '$fg' - [$title]"
        Say '  (that is not us: a VM / remote-desktop window swallows synthesized keys)'
        return "SKIPPED_KEYBOARD_GRABBED: $text"
    }
    return $text
}

# Overlay title ("Cuter " + jietu) and one app stage label, built from code points.
$titleShot = 'Cuter ' + [char]0x622A + [char]0x56FE
# "qing qiu yin cang zi shen chuang kou" = "request to hide our own windows"
$stageHide = -join @(0x8BF7, 0x6C42, 0x9690, 0x85CF, 0x81EA, 0x8EAB, 0x7A97, 0x53E3 |
    ForEach-Object { [char]$_ })

# ---------- 0. clean up + back up the config ----------
$stale = @(Get-Process cuter -ErrorAction SilentlyContinue)
if ($stale.Count -gt 0) {
    Say "killing $($stale.Count) stale cuter process(es) - they would steal the Alt+C hotkey"
    $stale | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}
Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force; Say "config backed up to $configBackup" }
New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
Remove-Item $outDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$config = @"
[general]
start_in_tray = true
close_to_tray = true
# Dark theme on purpose: the settings panel must differ from the light Notepad page as
# much as possible, so "is our window in the shot?" has a huge signal (dev ~200) to
# compare against the noise floor (a few units). With the light theme the window and the
# page look alike (dev ~25) and the test would be ambiguous.
theme = "dark"

[capture]
hide_own_windows = true
delay_ms = 0
play_sound = false
include_cursor = false
auto_detect_windows = false
show_magnifier = false

[save]
dir = '$outDir'
format = "png"
auto_save = true
ask_path_on_save = false
auto_copy = false
reveal_after_save = false

[ocr]
auto_after_capture = false
engine = "disabled"
"@
# Windows paths in TOML must be literal strings (single quotes): `dir = "E:\x"` is
# not a legal escape, the parser rejects the whole file, and the app silently falls
# back to defaults (which is how an earlier test "passed" without saving anything).
Set-Content -Path $configPath -Value $config -Encoding UTF8
Remove-Item (Join-Path (Split-Path $configPath) 'config.toml.bak') -Force -ErrorAction SilentlyContinue

# ---------- 1. static background: Notepad at a fixed place ----------
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null
$awareness = [CuterInputProbe]::Awareness()
Say "probe thread awareness = $awareness (0=unaware, 1=system, 2=per-monitor); screen=$([CuterInputProbe]::ScreenSize())"

$lines = 1..40 | ForEach-Object { "CUTER HIDE-OWN-WINDOWS E2E LINE {0:D3} ====" -f $_ }
Set-Content -Path $noteFile -Value $lines -Encoding UTF8
$notepad = Start-Process notepad.exe -ArgumentList $noteFile -PassThru
Start-Sleep -Seconds 2
$npHwnd = [CuterInputProbe]::FindWindow([uint32]$notepad.Id, 'cuter-hide-target')
if ($npHwnd -eq [IntPtr]::Zero) { $npHwnd = [CuterInputProbe]::FindWindow([uint32]$notepad.Id, 'Notepad') }
if ($npHwnd -eq [IntPtr]::Zero) { throw 'notepad window not found' }
[CuterInputProbe]::MoveWindow($npHwnd, 900, 260, 900, 700, $true) | Out-Null
Start-Sleep -Milliseconds 700

# R is the Notepad CLIENT area: static text, and it keeps the window title bar (whose
# colour flips between active/inactive) out of the comparison.
$clientInfo = @(([CuterInputProbe]::ClientInfo($npHwnd)) -split '\|')
$clientOrigin = @($clientInfo[0] -split ',')
$regionX = [int]$clientOrigin[0]
$regionY = [int]$clientOrigin[1]
$regionW = [int](($clientInfo[1] -split 'x')[0])
$regionH = [int](($clientInfo[1] -split 'x')[1])
Say "notepad client region R (probe units): ($regionX,$regionY) ${regionW}x${regionH}"
if ($regionW -lt 200 -or $regionH -lt 200) { throw "client region is degenerate: ${regionW}x${regionH}" }

# The overlay's client area sits 1px lower than its window rect (docs/HANDOFF.md 11.4),
# so probe-unit -> physical conversion is only needed for the headless `--region` CLI.
$factor = 1.0
if ($awareness -eq 0) {
    $factor = 1.25
    Say "WARNING: could not make this thread DPI aware - assuming 125% scaling"
}
function To-Physical($v) { return [int][math]::Round($v * $factor) }
$regionArg = "{0},{1},{2},{3}" -f (To-Physical $regionX), (To-Physical $regionY), (To-Physical $regionW), (To-Physical $regionH)

# ---------- 2. reference capture (nothing of ours on screen) ----------
$refPath = Join-Path $outDir 'reference.png'
& $exe --region $regionArg --out $refPath | Out-Null
if (-not (Test-Path $refPath)) { throw "reference capture failed: $refPath" }
Say "reference capture: $refPath ($regionArg)"

# ---------- 3. tray instance, then ask it to show the settings window ----------
$cuter = Start-Process $exe -PassThru -RedirectStandardError $logPath
Start-Sleep -Seconds 3

$rejected = Join-Path (Split-Path $configPath) 'config.toml.bak'
if (Test-Path $rejected) { throw "app rejected the test config (found $rejected) - check TOML syntax/values" }
Say "config accepted by the app"

# A second launch hands "show the settings window" to the running instance.
& $exe | Out-Null
$settings = [IntPtr]::Zero
foreach ($i in 1..40) {
    Start-Sleep -Milliseconds 250
    $settings = [CuterInputProbe]::FindWindow([uint32]$cuter.Id, 'Cuter')
    if ($settings -ne [IntPtr]::Zero) { break }
}
if ($settings -eq [IntPtr]::Zero) { throw 'settings window never appeared after the show handover' }
Say "settings window: $([CuterInputProbe]::Describe($settings))"

# Cover R exactly with our own window and let it paint.
[CuterInputProbe]::MoveWindow($settings, $regionX, $regionY, $regionW, $regionH, $true) | Out-Null
Start-Sleep -Milliseconds 900
$moved = @(([CuterInputProbe]::Describe($settings)) -split '\|')
Say "settings window moved onto R: x=$($moved[1]) y=$($moved[2]) w=$($moved[3]) h=$($moved[4]) visible=$($moved[5])"

# ---------- 4. teeth: R must now differ a lot from the reference ----------
$withPath = Join-Path $outDir 'with-window.png'
& $exe --region $regionArg --out $withPath | Out-Null
if (-not (Test-Path $withPath)) { throw "second capture failed: $withPath" }

Add-Type -AssemblyName System.Drawing

# Mean absolute per-channel deviation of `a` against `b`, shifted by `dy` rows.
function Get-MeanDeviation($a, $b, $dy) {
    $sum = 0.0; $n = 0
    for ($y = 2; $y -lt ($a.Height - 2); $y += 3) {
        $by = $y + $dy
        if ($by -lt 0 -or $by -ge $b.Height) { continue }
        for ($x = 0; $x -lt $a.Width; $x += 5) {
            $pa = $a.GetPixel($x, $y)
            $pb = $b.GetPixel($x, $by)
            $sum += [math]::Abs($pa.R - $pb.R) + [math]::Abs($pa.G - $pb.G) + [math]::Abs($pa.B - $pb.B)
            $n += 3
        }
    }
    return $sum / [math]::Max($n, 1)
}

function Get-BestDeviation($a, $b) {
    $best = [double]::MaxValue; $bestDy = 0
    foreach ($dy in -3..3) {
        $d = Get-MeanDeviation $a $b $dy
        if ($d -lt $best) { $best = $d; $bestDy = $dy }
    }
    return @($best, $bestDy)
}

$ref = [System.Drawing.Bitmap]::FromFile($refPath)
$with = [System.Drawing.Bitmap]::FromFile($withPath)
Say "reference $($ref.Width)x$($ref.Height)   with-window $($with.Width)x$($with.Height)"

$resWith = Get-BestDeviation $with $ref
$devWith = [double]$resWith[0]
Say ("deviation(with settings window vs reference) = {0:N3} (best dy={1})" -f $devWith, $resWith[1])
$with.Dispose()

if ($devWith -lt 12.0) {
    $ref.Dispose()
    throw "region R does not contain the settings window (dev=$devWith) - the test would pass vacuously"
}
# ---------- 5. Alt+C, select R, Enter (auto save) ----------
# The press is retried: the app logs its first line before `RegisterHotKey` has
# necessarily run, and a press that lands in that window is simply lost.
$overlay = [IntPtr]::Zero
foreach ($attempt in 1..3) {
    [CuterInputProbe]::AltKey(0x43)   # Alt+C
    foreach ($i in 1..30) {
        Start-Sleep -Milliseconds 250
        $overlay = [CuterInputProbe]::FindWindow([uint32]$cuter.Id, $titleShot)
        if ($overlay -ne [IntPtr]::Zero) { break }
    }
    if ($overlay -ne [IntPtr]::Zero) { break }
    if ($attempt -lt 3) {
        Say 'no overlay yet, pressing Alt+C again'
        Start-Sleep -Milliseconds 800
    }
}
if ($overlay -eq [IntPtr]::Zero) { throw (Blocked-Message 'overlay did not appear after Alt+C (3 tries)') }
$oRect = @(([CuterInputProbe]::Describe($overlay)) -split '\|')
Say "overlay: x=$($oRect[1]) y=$($oRect[2]) w=$($oRect[3]) h=$($oRect[4])"

# ⚠ The overlay window is created at `physical x system-scale` and only a pass or two later
# does eframe's GeometryFix pin pixels_per_point to 1.0 (docs/HANDOFF.md 11.4). Dragging
# during that transient maps screen pixels through the wrong scale - measured once as a
# 735x524 saved image instead of 882x628. Wait until the reported size equals the screen.
$screen = [CuterInputProbe]::ScreenSize()
$settled = $false
foreach ($i in 1..50) {
    Start-Sleep -Milliseconds 100
    $rect = @(([CuterInputProbe]::Describe($overlay)) -split '\|')
    if ("$($rect[3])x$($rect[4])" -eq $screen) { $settled = $true; break }
}
$oRect = @(([CuterInputProbe]::Describe($overlay)) -split '\|')
Say "overlay after settle: x=$($oRect[1]) y=$($oRect[2]) w=$($oRect[3]) h=$($oRect[4]) (screen=$screen, settled=$settled)"
if (-not $settled) { throw "overlay geometry never settled to the screen size (got $($oRect[3])x$($oRect[4]))" }

# The overlay must hold the keyboard before the drag: if the notepad keeps focus the press
# starts a text selection there and the Enter below never reaches the overlay (observed as
# "no PNG was auto-saved" on 2026-10-17).
[void][CuterInputProbe]::Activate($overlay)
Start-Sleep -Milliseconds 300
[CuterInputProbe]::Drag($regionX, $regionY, ($regionX + $regionW), ($regionY + $regionH))
Start-Sleep -Milliseconds 400
[CuterInputProbe]::Key(0x0D)   # Enter = save (save.auto_save = true)
Start-Sleep -Seconds 3

# Our settings window must be hidden again once the capture is over.
$after = @(([CuterInputProbe]::Describe($settings)) -split '\|')
Say "settings window after the capture: visible=$($after[5])"

# Third capture of the same region, now that the flow is over and our window is hidden again.
# This measures the BACKGROUND'S OWN DRIFT: the Notepad is static text, but its rendering
# (caret, DPI-related re-rasterisation, focus) is not perfectly frozen in time. Comparing
# `saved` against this one and against `reference` tells a leak apart from drift.
$afterPath = Join-Path $outDir 'after.png'
& $exe --region $regionArg --out $afterPath | Out-Null
if (-not (Test-Path $afterPath)) { throw "third capture failed: $afterPath" }

# ---------- 6. content assertion ----------
$pngs = @(Get-ChildItem $outDir -Filter *.png -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notin @('reference.png', 'with-window.png', 'after.png') })
if (-not $pngs) { throw "no PNG was auto-saved into $outDir" }
$png = $pngs | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$saved = [System.Drawing.Bitmap]::FromFile($png.FullName)
Say "saved: $($png.Name) = $($saved.Width)x$($saved.Height) ($($png.Length) bytes)"

# A wrong-size saved image means the drag went through a different scale than the reference
# capture (see the geometry-settle step above) - comparing the two would then be meaningless.
$expectW = To-Physical $regionW
$expectH = To-Physical $regionH
Say "expected saved size: ${expectW}x${expectH}"
$savedW = $saved.Width
$savedH = $saved.Height
$sizeOk = ([math]::Abs($savedW - $expectW) -le 3) -and ([math]::Abs($savedH - $expectH) -le 3)

$resSaved = Get-BestDeviation $saved $ref
$devSaved = [double]$resSaved[0]
Say ("deviation(saved selection vs reference) = {0:N3} (best dy={1})" -f $devSaved, $resSaved[1])
$saved.Dispose(); $ref.Dispose()

# How much does the background drift on its own between two window-free captures?
$before = [System.Drawing.Bitmap]::FromFile($refPath)
$after = [System.Drawing.Bitmap]::FromFile($afterPath)
$savedAgain = [System.Drawing.Bitmap]::FromFile($png.FullName)
$drift = [double](Get-BestDeviation $after $before)[0]
$devSavedAfter = [double](Get-BestDeviation $savedAgain $after)[0]
Say ("deviation(after vs reference) = {0:N3}  <- background drift between two window-free captures" -f $drift)
Say ("deviation(saved vs after)     = {0:N3}" -f $devSavedAfter)
$before.Dispose(); $after.Dispose(); $savedAgain.Dispose()

# The stage log is the "why": the app must have decided to wait for its own window
# to disappear before capturing. The pixel test above is the "what".
$hideHits = 0
if (Test-Path $logPath) {
    $hideHits = @(Get-Content $logPath -Encoding UTF8 | Select-String -SimpleMatch $stageHide).Count
}
Say "app stage lines 'hide our own windows' = $hideHits"

Stop-Process -Id $cuter.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $notepad.Id -Force -ErrorAction SilentlyContinue
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force } else { Remove-Item $configPath -Force -ErrorAction SilentlyContinue }

$fail = @()
# With the dark theme our window's own signature is ~200 (see the "with-window" line).
# The saved frame is never bit-identical to the reference: when a covered window is
# revealed again, DWM re-composites it and the text is rendered slightly differently for
# a moment (that residue is TEXT-shaped and is exactly 0 over the blank half of the
# region, while our window covers the region uniformly). Measured residue: 10~12.
# So the limit sits far below the window signal and far above the residue.
$leakLimit = 60.0
Say ("leak limit = {0:N1} (window signal above should be far larger; residue is text-only)" -f $leakLimit)
if (-not $sizeOk) { $fail += "saved image is ${savedW}x${savedH}, expected ${expectW}x${expectH} - the selection did not match the region" }
if ($devSaved -gt $leakLimit) { $fail += "saved selection still contains our own window (dev=$devSaved > $leakLimit)" }
if ($devSavedAfter -gt $leakLimit) { $fail += "saved selection differs from the window-free 'after' capture (dev=$devSavedAfter > $leakLimit)" }
if ($devWith -lt 60.0) { $fail += "premise broken: the region does not show our window strongly enough (dev=$devWith)" }
if ($hideHits -lt 1) { $fail += "app never took the wait-for-hide path (stage line count = $hideHits)" }
if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw "hide_own_windows e2e failed"
}
Say ("RESULT: PASS - our own window is gone from the shot (dev {0:N3}; {1:N3} when it was visible)" -f $devSaved, $devWith)
