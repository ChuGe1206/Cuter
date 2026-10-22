# Screenshot-history end-to-end test: capture for real, then check what landed on disk.
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\history_e2e.ps1
#
# What it proves (all against the real app, real captures, real files):
#
#   1. a delivered capture creates history: index.toml + images/<id>.png + thumbs/<id>.png;
#   2. the stored "original" is BIT-IDENTICAL to the PNG the same capture saved
#      (this is the difference between a history and a thumbnail gallery - the whole
#      point is that "copy it again" gives the pixels you captured);
#   3. the recorded byte count equals the real file size;
#   4. `history.limit` really prunes: the oldest entry AND its files disappear;
#   5. `history.limit = 0` (history off) writes nothing at all.
#
# It back up the user's config and history directory and restores them in `finally`.
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

# Scratch paths MUST be absolute (a relative one would land in the repository).
$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-history-config-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
# dirs::data_dir() on Windows is %LOCALAPPDATA% (a cache belongs in Local, not Roaming).
$historyDir = Join-Path $env:LOCALAPPDATA 'Cuter\history'
$historyBackup = Join-Path $tempRoot ("cuter-history-backup-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$saveDir = Join-Path $tempRoot ("cuter-history-saves-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$logDir = Join-Path $env:APPDATA 'Cuter\logs'
$logPath = Join-Path $logDir 'cuter.log'

if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force }
if (Test-Path $historyDir) { Copy-Item $historyDir $historyBackup -Recurse -Force }
New-Item -ItemType Directory -Force -Path $saveDir | Out-Null

function Stop-AllCuter {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 100
        if (-not (Get-Process cuter -ErrorAction SilentlyContinue)) { return }
    }
    throw 'a cuter process is still running'
}

trap {
    if ($script:blocked -or $_.Exception.Message -like 'SKIPPED_KEYBOARD_GRABBED*') {
        Write-Host "SKIPPED: $($_.Exception.Message -replace '^SKIPPED_KEYBOARD_GRABBED: ', '')"
    } else {
        Write-Host "FAILED: $($_.Exception.Message)"
    }
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
    if ($configBackup -and (Test-Path $configBackup)) { Copy-Item $configBackup $cfg -Force }
    $hist = Join-Path $env:LOCALAPPDATA 'Cuter\history'
    Remove-Item $hist -Recurse -Force -ErrorAction SilentlyContinue
    if ($historyBackup -and (Test-Path $historyBackup)) { Copy-Item $historyBackup $hist -Recurse -Force }
    # Exit code 2 = "could not run here", distinct from 1 = "the app is broken".
    if ($script:blocked -or $_.Exception.Message -like 'SKIPPED_KEYBOARD_GRABBED*') {
        Write-Host 'RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive'
        exit 2
    }
    exit 1
}

function Write-Config([int]$limit) {
    New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
    Set-Content -Path $configPath -Encoding UTF8 -Value @"
[general]
start_in_tray = true
close_to_tray = true

[capture]
hide_own_windows = true
delay_ms = 0
include_cursor = false
auto_detect_windows = false
show_magnifier = false

[save]
dir = '$saveDir'
format = "png"
auto_save = true
ask_path_on_save = false
auto_copy = false
reveal_after_save = false

[history]
limit = $limit
thumb_width = 200

[ocr]
engine = "disabled"
"@
}

# Read the index back as TOML-ish data with a tiny line parser (no TOML module in PS 5.1).
function Get-IndexEntries {
    $index = Join-Path $historyDir 'index.toml'
    if (-not (Test-Path $index)) { return @() }
    $text = Get-Content $index -Raw -Encoding UTF8
    $entries = @()
    $current = $null
    foreach ($line in ($text -split "`r?`n")) {
        if ($line -match '^\s*\[\[entries\]\]') {
            if ($current) { $entries += $current }
            $current = @{}
            continue
        }
        if ($null -ne $current -and $line -match '^\s*([a-z_]+)\s*=\s*(.+?)\s*$') {
            $value = $Matches[2].Trim('"')
            $current[$Matches[1]] = $value
        }
    }
    if ($current) { $entries += $current }
    return $entries
}

function Start-App {
    Stop-AllCuter
    $before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    $out = Join-Path $tempRoot 'cuter-history.out'
    $err = Join-Path $tempRoot 'cuter-history.err'
    $p = Start-Process $exe -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
    # Wait until the app really ran (it always logs the font it picked). Generous: a cold
    # start on a loaded machine can take tens of seconds.
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 60000) {
        if ((Test-Path $logPath) -and ((Get-Item $logPath).Length -gt $before)) { return $p }
        if ((Test-Path $err) -and (Get-Content $err -Raw -ErrorAction SilentlyContinue)) { return $p }
        $p.Refresh()
        if ($p.HasExited) { throw "the app exited immediately (exit code $($p.ExitCode))" }
        Start-Sleep -Milliseconds 200
    }
    throw 'the app never started within 60 s'
}

$titleShot = 'Cuter ' + [char]0x622A + [char]0x56FE

# ---------------------------------------------------------------------------
# Keyboard-grab guard
#
# A running VM window (VirtualBox / VMware) or a remote-desktop client takes the keyboard
# for itself: synthesized Alt+C and Enter then reach nobody, so the overlay never appears
# (or a selection is never confirmed) and the probe fails with a message that looks like
# an app bug. It is not - `cuter --capture` (single-instance forwarding, no keyboard)
# still works. So when every retry failed, check who owns the foreground; if it is not
# something of ours, report SKIPPED (exit code 2) instead of FAILED.
# ---------------------------------------------------------------------------
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

# One real capture: Alt+C -> wait for the overlay -> wait for its geometry to settle ->
# drag a region -> Enter (auto_save writes the file, which is what feeds the history).
#
# The hotkey is retried: `Start-App` returns as soon as the app logs anything, but
# `RegisterHotKey` can happen a moment later (the first log line is the font probe).
# A key press that lands in that window is simply lost, and no overlay ever appears -
# which used to surface as "overlay did not appear for capture 1" and looked like a bug
# in the app. Pressing again is harmless: if the overlay is already up we see it below.
function Invoke-Capture([int]$index) {
    $overlay = [IntPtr]::Zero
    foreach ($attempt in 1..3) {
        [CuterInputProbe]::AltKey(0x43)
        foreach ($i in 1..30) {
            Start-Sleep -Milliseconds 150
            $overlay = [CuterInputProbe]::FindWindow([uint32]$script:appPid, $titleShot)
            if ($overlay -ne [IntPtr]::Zero) { break }
        }
        if ($overlay -ne [IntPtr]::Zero) { break }
        if ($attempt -lt 3) {
            Say ("   (capture {0}: no overlay yet, pressing the hotkey again)" -f $index)
            Start-Sleep -Milliseconds 800
        }
    }
    if ($overlay -eq [IntPtr]::Zero) { throw (Blocked-Message "overlay did not appear for capture $index after 3 tries") }
    $screen = [CuterInputProbe]::ScreenSize()
    $settled = $false
    foreach ($i in 1..50) {
        Start-Sleep -Milliseconds 100
        $rect = @(([CuterInputProbe]::Describe($overlay)) -split '\|')
        if ("$($rect[3])x$($rect[4])" -eq $screen) { $settled = $true; break }
    }
    if (-not $settled) { throw "overlay geometry never settled for capture $index" }

    $x = 200 + $index * 30
    $y = 200 + $index * 20
    [CuterInputProbe]::Drag($x, $y, ($x + 260), ($y + 180))
    Start-Sleep -Milliseconds 300
    [CuterInputProbe]::Key(0x0D)   # Enter = save (auto_save = true -> also feeds history)
    Start-Sleep -Seconds 2
}

$fail = @()
Stop-AllCuter
Remove-Item $historyDir -Recurse -Force -ErrorAction SilentlyContinue

# ---------- A. limit = 2, three captures -> two entries, oldest pruned ----------
Write-Config 2
$app = Start-App
$script:appPid = [uint32]$app.Id
# Take the keyboard back from whatever is in front (a running VM window grabs it while
# active, and then synthesized Alt+C / Enter reach nobody).
[void][CuterInputProbe]::FocusDesktop()
Start-Sleep -Milliseconds 500
for ($i = 1; $i -le 3; $i++) { Invoke-Capture $i }

$entries = @(Get-IndexEntries)
Say "A. entries in index: $($entries.Count) (limit was 2)"
if ($entries.Count -ne 2) { $fail += "A: expected 2 entries after 3 captures with limit=2, got $($entries.Count)" }

$images = @(Get-ChildItem (Join-Path $historyDir 'images') -Filter *.png -ErrorAction SilentlyContinue)
$thumbs = @(Get-ChildItem (Join-Path $historyDir 'thumbs') -Filter *.png -ErrorAction SilentlyContinue)
Say "A. images=$($images.Count) thumbs=$($thumbs.Count)"
if ($images.Count -ne 2) { $fail += "A: expected 2 image files, got $($images.Count)" }
if ($thumbs.Count -ne 2) { $fail += "A: expected 2 thumb files, got $($thumbs.Count)" }

# The pruned entry's files must be gone (otherwise the cache grows forever)
foreach ($entry in $entries) {
    $image = Join-Path $historyDir ("images\{0}.png" -f $entry['id'])
    $thumb = Join-Path $historyDir ("thumbs\{0}.png" -f $entry['id'])
    if (-not (Test-Path $image)) { $fail += "A: index lists id=$($entry['id']) but $image is missing" }
    if (-not (Test-Path $thumb)) { $fail += "A: thumb missing for id=$($entry['id'])" }
    if (Test-Path $image) {
        $actual = (Get-Item $image).Length
        if ([int]$entry['bytes'] -ne $actual) {
            $fail += "A: id=$($entry['id']) records bytes=$($entry['bytes']) but the file is $actual"
        }
    }
}

# ---------- B. the stored original IS the original ----------
# Compare the newest history image with a real PNG of the same region captured straight
# from the screen: they must be byte-identical (same crop, same encoder).
$newest = $entries | Sort-Object { [int]$_['id'] } -Descending | Select-Object -First 1
if ($newest) {
    $savedFiles = @(Get-ChildItem $saveDir -Filter *.png | Sort-Object LastWriteTime -Descending)
    Say "B. auto-saved files: $($savedFiles.Count)"
    $historyImage = Join-Path $historyDir ("images\{0}.png" -f $newest['id'])
    $match = $savedFiles | Where-Object {
        (Get-FileHash $_.FullName -Algorithm SHA256).Hash -eq
        (Get-FileHash $historyImage -Algorithm SHA256).Hash
    } | Select-Object -First 1
    if ($match) {
        Say "B. history image is byte-identical to the auto-saved file $($match.Name)"
    } else {
        $fail += 'B: no auto-saved PNG matches the newest history image byte-for-byte'
    }
}

# ---------- D. the panel really opens ----------
# `cuter --history` is forwarded to the running instance (single-instance handover), so
# this also proves the new Activation::History path works end to end.
$titlePanel = 'Cuter ' + [char]0x622A + [char]0x56FE + [char]0x5386 + [char]0x53F2
& $exe --history | Out-Null
$panel = [IntPtr]::Zero
foreach ($i in 1..60) {
    Start-Sleep -Milliseconds 250
    $panel = [CuterInputProbe]::FindWindow([uint32]$app.Id, $titlePanel)
    if ($panel -ne [IntPtr]::Zero) { break }
}
Say "D. history panel window found: $($panel -ne [IntPtr]::Zero)"
if ($panel -eq [IntPtr]::Zero) {
    $fail += 'D: the history panel window never appeared after cuter --history'
} else {
    Say "   pane: $([CuterInputProbe]::Describe($panel))"
}

Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

# ---------- C. limit = 0 means no history at all ----------
Remove-Item $historyDir -Recurse -Force -ErrorAction SilentlyContinue
Write-Config 0
$app2 = Start-App
$script:appPid = [uint32]$app2.Id
Invoke-Capture 4
Stop-Process -Id $app2.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

$entriesAfter = @(Get-IndexEntries)
$imagesAfter = @(Get-ChildItem (Join-Path $historyDir 'images') -Filter *.png -ErrorAction SilentlyContinue)
Say "C. with limit = 0: entries=$($entriesAfter.Count) images=$($imagesAfter.Count)"
if ($entriesAfter.Count -ne 0 -or $imagesAfter.Count -ne 0) {
    $fail += 'C: history.limit = 0 still wrote history'
}

# ---------- cleanup ----------
Stop-AllCuter
$cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
if (Test-Path $configBackup) { Copy-Item $configBackup $cfg -Force }
Remove-Item $historyDir -Recurse -Force -ErrorAction SilentlyContinue
if (Test-Path $historyBackup) { Copy-Item $historyBackup $historyDir -Recurse -Force }
Remove-Item $saveDir -Recurse -Force -ErrorAction SilentlyContinue

if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw 'history e2e failed'
}
Say 'RESULT: PASS - captures land in history, originals are the originals, limit prunes, 0 disables'
