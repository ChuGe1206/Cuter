# paddle_ocr_e2e.ps1 - run the online OCR path against a *real* PaddleOCR server.
#
# The other OCR probes use tools/probe/fake_ocr_server.py, which answers with a fixed
# payload: they prove the request path works, not that a real engine can be read.
# This one starts the local PaddleOCR service (PaddleOCR 2.9.1 in a venv outside the
# repository, see tools/probe/README.md) and drives the app against it.
#
# It SKIPs (exit 2) when the venv or the model is not available, so a fresh clone or a
# machine without PaddleOCR still gets a green suite - the check is opt-in by installing
# the venv, never a hard requirement.
#
# NOTE: this file is deliberately ASCII-only. A BOM-less UTF-8 .ps1 with non-ASCII
# comments is parsed as GBK by Windows PowerShell 5.1, which eats the newline after a
# multi-byte sequence and makes the next line vanish into the comment.
#
# Usage:
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\probe\paddle_ocr_e2e.ps1
#   ... -Python C:\path\to\venv\Scripts\python.exe -Port 8917 -TimeoutSec 180
# Default interpreter: $env:CUTER_PADDLE_PYTHON, else %USERPROFILE%\paddle-ocr-venv\Scripts\python.exe

param(
    [string]$Python = $(if ($env:CUTER_PADDLE_PYTHON) { $env:CUTER_PADDLE_PYTHON } else { Join-Path $env:USERPROFILE 'paddle-ocr-venv\Scripts\python.exe' }),
    [int]$Port = 8917,
    [int]$TimeoutSec = 180
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$probe = $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$work = Join-Path $env:TEMP "cuter-paddle-$stamp"
New-Item -ItemType Directory -Path $work -Force | Out-Null
$serverOut = Join-Path $work 'server.out'
$serverErr = Join-Path $work 'server.err'
$requests = Join-Path $work 'requests.log'

function Skip([string]$why) {
    Write-Host "SKIPPED: $why"
    exit 2
}

function Fail([string]$why) {
    Write-Host "FAILED: $why"
    exit 1
}

$server = $null
try {
    if (-not (Test-Path $Python)) { Skip "no PaddleOCR venv at $Python (see tools\probe\README.md)" }

    Write-Host "starting the real PaddleOCR server on port $Port"
    $env:PYTHONIOENCODING = 'utf-8'
    $server = Start-Process -FilePath $Python -PassThru -WindowStyle Hidden `
        -ArgumentList @((Join-Path $probe 'paddle_ocr_server.py'), '--port', "$Port", '--log', $requests) `
        -RedirectStandardOutput $serverOut -RedirectStandardError $serverErr

    # Loading the model takes a few seconds; the very first run also downloads it.
    $ready = $false
    foreach ($i in 1..($TimeoutSec * 2)) {
        Start-Sleep -Milliseconds 500
        if ($server.HasExited) { break }
        if ((Test-Path $serverOut) -and (Select-String -Path $serverOut -Pattern 'model ready' -Quiet)) { $ready = $true; break }
    }
    if (-not $ready) {
        $tail = ''
        if (Test-Path $serverErr) { $tail = (Get-Content $serverErr -Tail 3) -join ' / ' }
        Skip "the PaddleOCR server did not come up within ${TimeoutSec}s ($tail)"
    }

    $endpoint = "http://127.0.0.1:$Port/ocr"
    Write-Host "server ready, driving the app against $endpoint"
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $probe 'ocr_http_e2e.ps1') `
        -Endpoint $endpoint -ServerLog $requests -Hotkey ocr
    if ($LASTEXITCODE -ne 0) { Fail "the app probe failed against the real server (exit $LASTEXITCODE)" }

    # The server writes the log line before it replies, but the file can still be a few
    # milliseconds behind the inner probe exiting, so poll for it.
    $line = $null
    foreach ($i in 1..20) {
        if (Test-Path $requests) {
            $line = Get-Content $requests -Encoding UTF8 | Where-Object { $_.Trim() -ne '' } | Select-Object -Last 1
        }
        if ($line -and $line -match '"blocks"') { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $line) { Fail 'the server never received a request' }

    # Only the blocks field matters, and the log carries UTF-8 text, so a plain regex is
    # steadier than a JSON parser here.
    if ($line -match '"blocks":\s*(\d+)') {
        $blocks = [int]$Matches[1]
    } else {
        Fail "the request log line is not the expected shape: $line"
    }
    if ($blocks -lt 3) { Fail "the server recognised only $blocks block(s) - the clipboard image probably had no text" }

    Write-Host ("the real engine returned {0} blocks" -f $blocks)
    Write-Host "RESULT: PASS - a real PaddleOCR server answered the online OCR path (artifacts in $work)"
    exit 0
}
catch {
    Fail $_.Exception.Message
}
finally {
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
}
