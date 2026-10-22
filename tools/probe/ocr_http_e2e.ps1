# End-to-end probe for the online (HTTP) OCR engine.
#
# `tools/probe/fake_ocr_server.py` listens on 127.0.0.1:$Port and answers like a PaddleOCR
# `hubserving` instance. The probe puts an image on the clipboard, configures the app to use
# the HTTP engine with the "pin from clipboard and OCR" hotkey, presses that hotkey, and then
# asserts that (a) the result panel appeared and (b) the fake service really received a
# multipart `image` field holding a base64 PNG.
#
# Why a hotkey instead of the capture overlay: driving the overlay with synthetic mouse and
# keyboard input proved flaky (Enter sometimes never reached it), while a registered global
# hotkey fires whatever has focus.
#
# ASCII only on purpose: Windows PowerShell 5.1 reads a BOM-less script as GBK, so Chinese
# text in comments swallows the next line break. Window titles are built from code points.
param(
    [int]$Port = 8899,
    [string]$Endpoint = '',
    [string]$Shot = '',
    [string]$Hotkey = 'ocr_clipboard',
    [string]$Python = '',
    [string]$ServerLog = ''
)
$ErrorActionPreference = 'Stop'
# One resolver for every probe that needs Python: `python` on PATH is often the Store alias
# stub (exists, exits 9009), so the helper validates each candidate with --version.
. (Join-Path $PSScriptRoot 'python.ps1')
if (-not $Python) { $Python = Require-Python -Why 'the fake server below' }
# PowerShell variable names are case-insensitive: the $endpoint assignment below wipes the
# $Endpoint parameter, so decide here whether we need the fake server.
$useFake = -not $Endpoint
$endpoint = if ($Endpoint) { $Endpoint } else { "http://127.0.0.1:$Port/ocr" }
$probe = $PSScriptRoot
$root = Split-Path -Parent (Split-Path -Parent $probe)
Set-Location $root
Add-Type -Path (Join-Path $probe 'CuterInputProbe.cs')
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null

$log = if ($ServerLog) { $ServerLog } else { Join-Path $root 'target\tmp\fake-ocr-requests.log' }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $log) | Out-Null
if (Test-Path $log) { Remove-Item $log -Force }

$cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
$backup = "$cfg.ocr-probe-backup"
if (Test-Path $backup) { Remove-Item $backup -Force }
if (Test-Path $cfg) { Copy-Item $cfg $backup -Force }

function Clean {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

$server = $null
try {
    Clean
    @"
[general]
theme = "dark"
start_in_tray = false

[hotkeys]
ocr_clipboard = "Alt+Shift+O"
ocr = "Alt+Shift+R"

[ocr]
engine = "http"
http_endpoint = "$endpoint"
auto_copy = false
paragraph = false
min_score = 0.3
"@ | Set-Content -Path $cfg -Encoding UTF8

    if ($useFake) {
        $fakeOut = Join-Path $root 'target\tmp\fake-ocr-server.out'
        $fakeErr = Join-Path $root 'target\tmp\fake-ocr-server.err'
        $server = Start-Process $Python -ArgumentList @(
            (Join-Path $probe 'fake_ocr_server.py'), "$Port", $log
        ) -PassThru -WindowStyle Hidden -RedirectStandardOutput $fakeOut -RedirectStandardError $fakeErr
        $ready = $false
        foreach ($i in 1..40) {
            Start-Sleep -Milliseconds 250
            try {
                $probeReply = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/ocr" -TimeoutSec 2
                if ($probeReply.StatusCode -eq 200) { $ready = $true; break }
            } catch { }
        }
        if (-not $ready) { throw 'the fake OCR server did not come up' }
        Write-Host "fake OCR server pid=$($server.Id) alive=$(-not $server.HasExited) port=$Port"
    } else {
        Write-Host "using an external OCR service at $endpoint (log: $log)"
    }

    # The clipboard image must contain text: the fake service does not care, but a real
    # PaddleOCR returns zero blocks for a landscape photo. Start a window with three big
    # text lines, grab it, put it on the clipboard, then close it.
    Add-Type -Path (Join-Path $probe 'OcrTarget.cs')
    $target = [OcrTarget]::Start(360, 260)
    Start-Sleep -Milliseconds 600
    $tRect = [CuterInputProbe]::WindowRect($target)
    $bitmap = [System.Drawing.Bitmap]::new($tRect.Right - $tRect.Left, $tRect.Bottom - $tRect.Top)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($tRect.Left, $tRect.Top, 0, 0, $bitmap.Size)
    $graphics.Dispose()
    [System.Windows.Forms.Clipboard]::SetImage($bitmap)
    # Move the target off screen once the pixels are gone: WM_CLOSE is queued, and the probe
    # screenshots the result panel right after, so a target still in place covers the panel's
    # bottom-right corner (that is how the first evidence shot came out).
    [void][CuterInputProbe]::MoveWindow($target, -4000, -4000, 820, 420, $false)
    [OcrTarget]::Stop()
    Write-Host "clipboard image: $($bitmap.Width)x$($bitmap.Height) (captured from the OCR target window)"

    $p = Start-Process (Join-Path $root 'target\release\cuter.exe') -PassThru
    $main = [IntPtr]::Zero
    foreach ($i in 1..60) {
        Start-Sleep -Milliseconds 250
        $main = [CuterInputProbe]::FindWindow([uint32]$p.Id, 'Cuter')
        if ($main -ne [IntPtr]::Zero) { break }
    }
    if ($main -eq [IntPtr]::Zero) { throw 'the main window did not appear' }
    Start-Sleep -Milliseconds 1500   # let the hotkeys get registered

    # Two hotkeys: ocr_clipboard = Alt+Shift+O (pin + OCR), ocr = Alt+Shift+R (OCR only).
    # Evidence shots use the latter: the former also opens an always-on-top pin that covers the
    # result panel's bottom-right corner.
    $keys = @{ 'ocr_clipboard' = 0x4F; 'ocr' = 0x52 }
    if (-not $keys.ContainsKey($Hotkey)) { throw "unknown -Hotkey '$Hotkey' (use ocr_clipboard or ocr)" }
    Write-Host ("pressing Alt+Shift+" + [char]$keys[$Hotkey] + " ($Hotkey)")
    [CuterInputProbe]::KeyDown(0x12)   # Alt
    [CuterInputProbe]::KeyDown(0x10)   # Shift
    [CuterInputProbe]::Key($keys[$Hotkey])
    [CuterInputProbe]::KeyUp(0x10)
    [CuterInputProbe]::KeyUp(0x12)

    $panelTitle = 'Cuter ' + [char]0x8BC6 + [char]0x522B + [char]0x7ED3 + [char]0x679C   # SHI BIE JIE GUO
    $panel = [IntPtr]::Zero
    foreach ($i in 1..60) {
        Start-Sleep -Milliseconds 250
        $panel = [CuterInputProbe]::FindWindow([uint32]$p.Id, $panelTitle)
        if ($panel -ne [IntPtr]::Zero) { break }
    }
    $rect = if ($panel -ne [IntPtr]::Zero) { [CuterInputProbe]::WindowRect($panel) } else { $null }
    if ($rect) {
        Write-Host "result panel: $($rect.Left),$($rect.Top) $($rect.Right - $rect.Left)x$($rect.Bottom - $rect.Top)"
    }
    if ($Shot -and $rect) {
        # Full screen, not just the panel rect: the app can still be moving the panel when we
        # read the rect, and a rect capture then lands on whatever was there before.
        $vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
        Start-Sleep -Milliseconds 1200
        $bitmap = [System.Drawing.Bitmap]::new($vs.Width, $vs.Height)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen(
            $vs.Left, $vs.Top, 0, 0, [System.Drawing.Size]::new($vs.Width, $vs.Height)
        )
        $graphics.Dispose()
        $bitmap.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
        $bitmap.Dispose()
        Write-Host "shot: $Shot (full screen $($vs.Width)x$($vs.Height); panel rect was $($rect.Left),$($rect.Top))"
    }

    $pass = $true
    if ($panel -eq [IntPtr]::Zero) { Write-Host 'FAIL: the OCR result panel did not appear'; $pass = $false }
    if (-not (Test-Path $log)) {
        Write-Host 'FAIL: the OCR service logged no request at all'
        $pass = $false
    } elseif (-not $useFake) {
        Write-Host 'the OCR service saw:'
        Get-Content $log | ForEach-Object { Write-Host "  $_" }
        $seen = Get-Content $log -Raw
        if ($seen -notmatch '"blocks":\s*[1-9]') {
            Write-Host 'FAIL: the real OCR service recognised no text at all'
            $pass = $false
        }
    } else {
        Write-Host 'the fake server saw:'
        Get-Content $log | ForEach-Object { Write-Host "  $_" }
        $seen = Get-Content $log -Raw
        if ($seen -notmatch 'has_image_field=True') { Write-Host 'FAIL: no multipart image field'; $pass = $false }
        if ($seen -notmatch 'has_png_base64=True') { Write-Host 'FAIL: the field carried no base64 PNG'; $pass = $false }
        if ($endpoint -match 'rotate=1' -and $seen -notmatch 'rotated=True') {
            Write-Host 'FAIL: asked for the rotated fixture but the server served the plain one'
            $pass = $false
        }
        if ($endpoint -match 'vertical=1' -and $seen -notmatch 'vertical=True') {
            Write-Host 'FAIL: asked for the vertical fixture but the server served another one'
            $pass = $false
        }
    }

    if ($pass) {
        Write-Host 'RESULT: PASS - the online engine posted the image and the result panel opened'
    } else {
        # throw (not exit) so the finally block still stops the app and restores the config
        throw 'RESULT: FAIL - the online OCR path did not complete'
    }
} finally {
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
    try { [OcrTarget]::Stop() } catch { }
    Clean
    if (Test-Path $backup) { Move-Item $backup $cfg -Force }
}
