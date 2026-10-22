# DXGI capture backend end to end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\dxgi_backend_e2e.ps1
#
# What it proves (all without touching the keyboard):
#   1. `capture.backend = "dxgi"` really is used by the headless capture path
#      (the log says so, and the PNG comes out sane: right size, not all black,
#      not all one colour);
#   2. the same capture with `capture.backend = "gdi"` produces a PNG of the same
#      size whose average brightness is within a small tolerance - i.e. both
#      backends are looking at the same desktop (the pixel-exact comparison lives
#      in the ignored Rust test `dxgi_vs_gdi_matches_and_times`, which can hold
#      both backends at once);
#   3. a bogus backend value in the config falls back to GDI instead of failing
#      (defensive: hand-edited configs);
#   4. the user's config is restored, whatever happens.
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

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }
$work = Join-Path $tempRoot ("cuter-dxgi-e2e-{0}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $work | Out-Null

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $work 'config-backup.toml'
Copy-Item $configPath $configBackup -Force
$logPath = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'

trap {
    Write-Host "FAILED: $($_.Exception.Message)"
    if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }
    Write-Host 'RESULT: FAIL - see above'
    exit 1
}

Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

function Set-Backend([string]$value) {
    $text = Get-Content $configPath -Raw -Encoding UTF8
    if ($text -match '(?m)^\s*backend\s*=') {
        $text = [regex]::Replace($text, '(?m)^\s*backend\s*=.*$', "backend = `"$value`"")
    } elseif ($text -match '(?m)^\[capture\]') {
        $text = [regex]::Replace($text, '(?m)^\[capture\]', "[capture]`nbackend = `"$value`"")
    } else {
        $text = "[capture]`nbackend = `"$value`"`n`n" + $text
    }
    [System.IO.File]::WriteAllText($configPath, $text, (New-Object System.Text.UTF8Encoding($false)))
}

function Measure-Png([string]$path) {
    $bmp = [System.Drawing.Bitmap]::FromFile($path)
    try {
        $sum = 0.0
        $count = 0
        $distinct = 0
        $seen = @{}
        for ($y = 0; $y -lt $bmp.Height; $y += 37) {
            for ($x = 0; $x -lt $bmp.Width; $x += 37) {
                $c = $bmp.GetPixel($x, $y)
                $sum += ($c.R + $c.G + $c.B) / 3.0
                $count++
                $key = "{0},{1},{2}" -f $c.R, $c.G, $c.B
                if (-not $seen.ContainsKey($key)) { $seen[$key] = $true; $distinct++ }
            }
        }
        return [pscustomobject]@{
            Width    = $bmp.Width
            Height   = $bmp.Height
            Mean     = if ($count -gt 0) { $sum / $count } else { 0 }
            Distinct = $distinct
            Samples  = $count
        }
    } finally { $bmp.Dispose() }
}

function Invoke-Capture([string]$label, [string]$outName) {
    $out = Join-Path $work $outName
    $before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $exe --full -o $out | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "$label : cuter --full exited with $LASTEXITCODE" }
    $elapsed = $sw.ElapsedMilliseconds
    if (-not (Test-Path $out)) { throw "$label : no output file at $out" }
    $new = ''
    if (Test-Path $logPath) {
        $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
        try {
            $stream.Seek($before, 'Begin') | Out-Null
            $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
            $new = $reader.ReadToEnd()
        } finally { $stream.Dispose() }
    }
    $png = Measure-Png $out
    Say ("{0,-12} {1,5} ms  {2}x{3}  mean {4:N1}  distinct colours {5}/{6}" -f `
        $label, $elapsed, $png.Width, $png.Height, $png.Mean, $png.Distinct, $png.Samples)
    return [pscustomobject]@{ Png = $png; Log = $new; Ms = $elapsed }
}

# --- 1. DXGI -----------------------------------------------------------------
Set-Backend 'dxgi'
$dxgi = Invoke-Capture 'dxgi' 'dxgi.png'
if ($dxgi.Log -notmatch 'DXGI') {
    throw "the DXGI run never mentioned DXGI in the log - the config was not honoured`n$($dxgi.Log)"
}
# The app logs the *requested* backend as `Dxgi` when it falls back, and the backend's
# own name (`Windows DXGI...`) when it really built one. Both markers are ASCII, which
# matters: a .ps1 without a BOM is read as GBK by Windows PowerShell, so a Chinese
# pattern would never match (and can break parsing outright).
if ($dxgi.Log -match 'Dxgi' -and $dxgi.Log -notmatch 'Windows DXGI') {
    Copy-Item $configBackup $configPath -Force
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Say 'DESKTOP DUPLICATION UNAVAILABLE: the log shows a fallback to GDI'
    Say 'RESULT: SKIPPED - this machine cannot provide DXGI desktop duplication right now'
    exit 2
}

# --- 2. GDI ------------------------------------------------------------------
Set-Backend 'gdi'
$gdi = Invoke-Capture 'gdi' 'gdi.png'
if ($gdi.Log -match 'DXGI') { throw 'the GDI run should not touch DXGI at all' }

# --- 3. compare --------------------------------------------------------------
if ($dxgi.Png.Width -ne $gdi.Png.Width -or $dxgi.Png.Height -ne $gdi.Png.Height) {
    throw "backends disagree on the desktop size: DXGI $($dxgi.Png.Width)x$($dxgi.Png.Height) vs GDI $($gdi.Png.Width)x$($gdi.Png.Height)"
}
$delta = [math]::Abs($dxgi.Png.Mean - $gdi.Png.Mean)
Say ("mean brightness delta: {0:N2} (threshold 12)" -f $delta)
if ($delta -gt 12) { throw "the two backends see clearly different pictures (mean brightness differs by $delta)" }
if ($dxgi.Png.Distinct -lt 5) { throw "the DXGI capture looks degenerate: only $($dxgi.Png.Distinct) distinct colours sampled" }
if ($dxgi.Png.Mean -lt 1) { throw 'the DXGI capture is essentially black' }

# --- 4. bogus value in a hand-edited config must not break capturing ---------
Set-Backend 'not-a-real-backend'
$fallback = Invoke-Capture 'bogus' 'bogus.png'
if ($fallback.Png.Width -ne $gdi.Png.Width) { throw 'the bogus backend value did not fall back to a working capture' }
Say 'bogus backend value -> fell back and still produced a full-size PNG'

Copy-Item $configBackup $configPath -Force
Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Say 'user config restored'
Say "RESULT: PASS - DXGI works end to end, agrees with GDI, and bogus config values fall back safely"
