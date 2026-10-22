# End-to-end probe for the update check (0.1.47).
#
# The startup check runs once per process, so every case below launches a fresh `cuter --about`
# (that flag parks the settings window on the About page, where the update card lives).
#
#   A newer - the fake manifest server answers 9.9.9: it must see `GET /update.json` with a
#             `UA=Cuter/<version>` header, the app log must gain an `[update]` line naming 9.9.9,
#             and we screenshot the About page for the visual check.
#   B off   - `check_update = false`: the server must see nothing and the app must log no
#             `[update]` line at all. This is the promise "empty config sends not a single byte".
#   C empty - `check_update = true` but `update_endpoint = ""`: same as B, the default config.
#   D https - a real https endpoint: the log must gain an `[update]` line WITHOUT 9.9.9, i.e. the
#             WinHTTP TLS leg completed and failed on the body (example.com is not a manifest)
#             instead of hanging or dying inside the TLS handshake.
#   E fresh - `last_update_check` written seconds ago: the startup check must stay silent (no
#             request, `throttled-start` in the log). Then the same config with a 25 h old stamp must
#             check again - the throttle has to expire, not swallow every future check.
#   F skip  - `skipped_version = 9.9.9` while the server offers 9.9.9: the request still leaves
#             (it is a check, not a prompt), but the log says the version is on the skip list and
#             the About card shows the skipped state instead of prompting.
#
# Case A also asserts that a successful check persists `last_update_check` in the config: that
# timestamp is what makes the daily throttle work across launches.
#
# ASCII only on purpose: Windows PowerShell 5.1 reads a BOM-less script as GBK, so Chinese text
# in here would swallow the next line break.
param(
    [int]$Port = 8901,
    [string]$Version = '9.9.9',
    # TEMP by default: run_all.ps1 must not overwrite the archived evidence shots.
    # To refresh them, pass -ShotDir docs\design\shots explicitly.
    [string]$ShotDir = 'target\tmp',
    [string]$Python = ''
)
$ErrorActionPreference = 'Stop'
# One resolver for every probe that needs Python: `python` on PATH is often the Store alias
# stub (exists, exits 9009), so the helper validates each candidate with --version.
. (Join-Path $PSScriptRoot 'python.ps1')
if (-not $Python) { $Python = Require-Python -Why 'the fake server below' }
$probe = $PSScriptRoot
$root = Split-Path -Parent (Split-Path -Parent $probe)
Set-Location $root
Add-Type -Path (Join-Path $probe 'CuterInputProbe.cs')
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[CuterInputProbe]::SetPhysicalDpiAwareness() | Out-Null

# The window's own surface, not the screen: a window that sits behind something else still
# paints, and CopyFromScreen would hand back a white rectangle for it.
Add-Type -ReferencedAssemblies System.Drawing, System.Windows.Forms -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class WindowShot
{
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);

    public static bool Save(IntPtr hwnd, int width, int height, string path)
    {
        if (width <= 0 || height <= 0) { return false; }
        using (var bmp = new Bitmap(width, height))
        using (var g = Graphics.FromImage(bmp))
        {
            IntPtr hdc = g.GetHdc();
            bool ok = PrintWindow(hwnd, hdc, 2);
            g.ReleaseHdc(hdc);
            if (!ok) { return false; }
            bmp.Save(path, ImageFormat.Png);
            return true;
        }
    }
}
'@

$log = Join-Path $root 'target\tmp\fake-update-requests.log'
$cfg = Join-Path $env:APPDATA 'Cuter\config.toml'
$appLog = Join-Path $env:APPDATA 'Cuter\logs\cuter.log'
$backup = "$cfg.update-probe-backup"
$exe = Join-Path $root 'target\release\cuter.exe'
$shots = Join-Path $root $ShotDir
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $log) | Out-Null
New-Item -ItemType Directory -Force -Path $shots | Out-Null

$script:pass = $true

function Fail([string]$why) {
    Write-Host "FAIL: $why"
    $script:pass = $false
}

function Clean {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
}

# Both logs start empty so each case only sees its own requests.
function Reset-Logs {
    if (Test-Path $log) { Remove-Item $log -Force }
    $dir = Split-Path -Parent $appLog
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    [System.IO.File]::WriteAllText($appLog, '')
}

function Write-Config([string]$check, [string]$endpoint, [string]$extra = '') {
    $text = @"
[general]
theme = "dark"
start_in_tray = false
show_tray_notifications = false
check_update = $check
update_endpoint = "$endpoint"
$extra
"@
    [System.IO.File]::WriteAllText($cfg, $text, [System.Text.UTF8Encoding]::new($false))
}

function Start-App {
    $process = Start-Process $exe -ArgumentList '--about' -PassThru
    $window = [IntPtr]::Zero
    foreach ($i in 1..60) {
        Start-Sleep -Milliseconds 250
        $window = [CuterInputProbe]::FindWindow([uint32]$process.Id, 'Cuter')
        if ($window -ne [IntPtr]::Zero) { break }
    }
    if ($window -eq [IntPtr]::Zero) { throw 'the settings window did not appear' }
    # A tall window so the whole About page (update card included) fits into one shot, and the
    # cursor parked on the title bar so no tooltip from whatever sits behind covers the card.
    [CuterInputProbe]::MoveWindow($window, 120, 60, 1100, 960, $true) | Out-Null
    Start-Sleep -Milliseconds 700
    $rect = [CuterInputProbe]::WindowRect($window)
    [CuterInputProbe]::MoveTo($rect.Left + 300, $rect.Top + 20) | Out-Null
    Start-Sleep -Milliseconds 400
    $script:appWindow = $window
    return $process
}

function Wait-AppLog([string]$pattern, [int]$seconds) {
    foreach ($i in 1..($seconds * 4)) {
        Start-Sleep -Milliseconds 250
        if ((Test-Path $appLog) -and ((Get-Content $appLog -Raw) -match $pattern)) { return $true }
    }
    return $false
}

function App-Log {
    if (Test-Path $appLog) { return (Get-Content $appLog -Raw) } else { return '' }
}

# Pick a port that really binds: dev machines often have something on 8901,
# and that collision must not look like "update check is broken".
function Find-FreePort([int]$Start) {
    for ($port = $Start; $port -lt ($Start + 20); $port++) {
        $listener = $null
        try {
            $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $port)
            $listener.Start()
            return $port
        } catch {
        } finally {
            if ($listener) { $listener.Stop() }
        }
    }
    return $Start
}

function Take-Shot([string]$path, [IntPtr]$window = [IntPtr]::Zero) {
    # A window that is behind something else still paints; grab its own surface first so the
    # evidence shot can never be a white rectangle.
    if ($window -ne [IntPtr]::Zero) {
        [CuterInputProbe]::SetForegroundWindow($window) | Out-Null
        Start-Sleep -Milliseconds 400
        $rect = [CuterInputProbe]::WindowRect($window)
        $w = $rect.Right - $rect.Left
        $h = $rect.Bottom - $rect.Top
        if ([WindowShot]::Save($window, $w, $h, $path)) {
            Write-Host "shot (window): $path"
            return
        }
    }
    $vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = [System.Drawing.Bitmap]::new($vs.Width, $vs.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen(
        $vs.Left, $vs.Top, 0, 0, [System.Drawing.Size]::new($vs.Width, $vs.Height)
    )
    $graphics.Dispose()
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    Write-Host "shot: $path"
}

$server = $null
try {
    Clean
    if (Test-Path $backup) { Remove-Item $backup -Force }
    if (Test-Path $cfg) { Copy-Item $cfg $backup -Force }

    # The port may be taken by another dev service (something on 8901 answers 401
    # here). An explicit -Port wins; otherwise pick the first port we can bind, so an
    # environment collision is never reported as a product bug.
    if (-not $PSBoundParameters.ContainsKey('Port')) {
        $Port = Find-FreePort $Port
        Write-Host "using port $Port for the fake update server"
    }

    # ---------------------------------------------------------------- case A: a newer manifest
    Reset-Logs
    Write-Config 'true' "http://127.0.0.1:$Port/update.json"
    $server = Start-Process $Python -ArgumentList @(
        (Join-Path $probe 'fake_update_server.py'), "$Port", $log, $Version
    ) -PassThru -WindowStyle Hidden
    $ready = $false
    foreach ($i in 1..40) {
        Start-Sleep -Milliseconds 250
        try {
            $reply = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/update.json" -TimeoutSec 2
            if ($reply.StatusCode -eq 200) { $ready = $true; break }
        } catch { }
    }
    if (-not $ready) { throw 'the fake update server did not come up' }
    Remove-Item $log -Force    # the readiness request is ours, not the app's

    $process = Start-App
    if (-not (Wait-AppLog '\[update\]' 15)) { Fail 'the app never logged an [update] line' }
    Start-Sleep -Milliseconds 1200
    $shotA = Join-Path $shots '0.1.47-about-newer.png'
    Take-Shot $shotA $script:appWindow
    if ((App-Log) -notmatch [regex]::Escape($Version)) {
        Fail "the app log does not name $Version"
    }
    if (-not (Test-Path $log)) {
        Fail 'the fake server saw no request at all'
    } else {
        $seen = (Get-Content $log -Raw).Trim()
        Write-Host "the fake server saw: $seen"
        if ($seen -notmatch 'GET /update\.json') { Fail 'the request path was not /update.json' }
        if ($seen -notmatch 'UA=Cuter/') { Fail 'the request carried no Cuter User-Agent' }
    }
    # A successful check must be remembered, otherwise the daily throttle never engages.
    $stamp = [regex]::Match([System.IO.File]::ReadAllText($cfg), 'last_update_check\s*=\s*(\d+)')
    if (-not $stamp.Success) {
        Fail 'the successful check did not persist last_update_check'
    } elseif ([int64]$stamp.Groups[1].Value -le 0) {
        Fail 'last_update_check stayed at zero after a successful check'
    } else {
        Write-Host "persisted last_update_check = $($stamp.Groups[1].Value)"
    }
    Clean

    # ---------------------------------------------------------------- case B: the switch is off
    Reset-Logs
    Write-Config 'false' "http://127.0.0.1:$Port/update.json"
    $process = Start-App
    Start-Sleep -Seconds 5
    Clean
    if (Test-Path $log) { Fail 'check_update = false still sent a request' }
    if ((App-Log) -match '\[update\]') { Fail 'check_update = false still ran the check' }

    # ---------------------------------------------------------------- case C: no endpoint yet
    Reset-Logs
    Write-Config 'true' ''
    $process = Start-App
    Start-Sleep -Seconds 5
    Clean
    if (Test-Path $log) { Fail 'an empty update_endpoint still sent a request' }
    if ((App-Log) -match '\[update\]') { Fail 'an empty update_endpoint still ran the check' }

    # ---------------------------------------------------------------- case D: a real https host
    Reset-Logs
    Write-Config 'true' 'https://example.com/'
    $process = Start-App
    if (-not (Wait-AppLog '\[update\]' 30)) { Fail 'the https check neither finished nor failed' }
    Start-Sleep -Milliseconds 1200
    $shotD = Join-Path $shots '0.1.47-about-check-failed.png'
    Take-Shot $shotD $script:appWindow
    if ((App-Log) -match [regex]::Escape($Version)) {
        Fail 'example.com cannot answer with our fixture version'
    }
    Clean

    # ---------------------------------------------------------------- case E: the daily throttle
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    Reset-Logs
    Write-Config 'true' "http://127.0.0.1:$Port/update.json" "last_update_check = $now"
    $process = Start-App
    Start-Sleep -Seconds 4
    Clean
    if (Test-Path $log) { Fail 'a check from minutes ago still sent a request' }
    if (-not (Wait-AppLog '\[update\] throttled' 6)) { Fail 'the throttled launch logged no reason' }

    # ... and the throttle has to expire: 25 hours later the check runs again.
    Reset-Logs
    $stale = $now - 25 * 3600
    Write-Config 'true' "http://127.0.0.1:$Port/update.json" "last_update_check = $stale"
    $process = Start-App
    if (-not (Wait-AppLog '\[update\]' 15)) { Fail 'a stale timestamp suppressed the check forever' }
    Start-Sleep -Milliseconds 800
    Clean
    if (-not (Test-Path $log)) { Fail 'a stale timestamp never reached the server' }

    # ---------------------------------------------------------------- case F: the skip list
    Reset-Logs
    Write-Config 'true' "http://127.0.0.1:$Port/update.json" "skipped_version = `"$Version`""
    $process = Start-App
    if (-not (Wait-AppLog '\[update\]' 15)) { Fail 'the skipped version never got checked' }
    Start-Sleep -Milliseconds 1500
    if (-not (Wait-AppLog '\[update\] skip-hit' 6)) {
        Fail 'a skipped version still prompted (no skip log line)'
    }
    $shotF = Join-Path $shots '0.1.69-about-skipped.png'
    Take-Shot $shotF $script:appWindow
    Clean

    if ($script:pass) {
        Write-Host 'RESULT: PASS - the check runs, is silent when off, throttles to once a day and honours the skip list'
    } else {
        throw 'RESULT: FAIL - the update check did not behave'
    }
} finally {
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
    Clean
    if (Test-Path $backup) { Move-Item $backup $cfg -Force }
}
