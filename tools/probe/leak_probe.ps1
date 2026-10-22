# Leak probe: hammer the capture path and watch the process's resource counters.
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\leak_probe.ps1 [-Cycles 60]
#
# Why this exists (roadmap M5-D): a screenshot tool is a long-running tray process, and
# the classic way such a tool dies is resource exhaustion - not memory. Every capture
# creates a screen DC, a memory DC and a DIB section; a missing DeleteDC/DeleteObject
# shows up in the GDI object count immediately, while the process only collapses at the
# 10 000-handle limit hours later. So this probe measures, after a warm-up:
#
#   * GDI objects   (GetGuiResources GR_GDIOBJECTS)
#   * USER objects  (GetGuiResources GR_USEROBJECTS)
#   * kernel handles (Process.HandleCount)
#   * private bytes  (Process.PrivateMemorySize64)
#
# and fails if any of them grows beyond a small tolerance.
#
# Each cycle is a real capture cancelled with Esc: the overlay is created, the whole
# virtual desktop is captured, the overlay is destroyed. No files are written.
param(
    [string]$Exe = '',
    [int]$Cycles = 60,
    [int]$Warmup = 5
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

$tempRoot = [System.IO.Path]::GetTempPath()
if (-not [System.IO.Path]::IsPathRooted($tempRoot)) { throw "temp path is not absolute: '$tempRoot'" }

$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
$configBackup = Join-Path $tempRoot ("cuter-leak-config-{0}.toml" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$logDir = Join-Path $env:APPDATA 'Cuter\logs'
$logPath = Join-Path $logDir 'cuter.log'

if (Test-Path $configPath) { Copy-Item $configPath $configBackup -Force }

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
    if ($configBackup -and (Test-Path $configBackup)) {
        Copy-Item $configBackup (Join-Path $env:APPDATA 'Cuter\config.toml') -Force
    }
    if ($script:blocked) {
        Write-Host 'RESULT: SKIPPED - another program owns the keyboard, so synthesized keys never arrive'
        exit 2
    }
    exit 1
}

# No save, no copy, no history: this probe is about the capture path only, and writing
# files would add unrelated filesystem noise to the memory numbers.
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
auto_save = false
auto_copy = false
reveal_after_save = false

[history]
limit = 0

[ocr]
engine = "disabled"
"@

function Start-App {
    Stop-AllCuter
    $before = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    $out = Join-Path $tempRoot 'cuter-leak.out'
    $err = Join-Path $tempRoot 'cuter-leak.err'
    $p = Start-Process $exe -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
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

function Get-Counters($proc) {
    $proc.Refresh()
    [pscustomobject]@{
        Gdi     = [CuterInputProbe]::GdiObjects([uint32]$proc.Id)
        User    = [CuterInputProbe]::UserObjects([uint32]$proc.Id)
        Handles = $proc.HandleCount
        Private = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
        Working = [math]::Round($proc.WorkingSet64 / 1MB, 1)
    }
}

function Show-Counters($label, $c) {
    Say ("{0,-10} GDI={1,-5} USER={2,-5} handles={3,-6} private={4,7} MB  working={5,7} MB" -f `
        $label, $c.Gdi, $c.User, $c.Handles, $c.Private, $c.Working)
}

$titleShot = 'Cuter ' + [char]0x622A + [char]0x56FE

# A VM window (VirtualBox / VMware) or a remote-desktop client grabs the keyboard, and
# then synthesized Alt+C / Esc never arrive. That looks like an app bug but is not, so
# report SKIPPED (exit 2) with the culprit instead of FAILED.
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

# One capture cycle: Alt+C, wait for the overlay, press Esc (cancel).
#
# The hotkey is retried for the same reason as in history_e2e.ps1: the app logs its first
# line before `RegisterHotKey` has necessarily run, so an early press is simply lost.
function Invoke-Cycle([int]$index) {
    $overlay = [IntPtr]::Zero
    foreach ($attempt in 1..3) {
        [CuterInputProbe]::AltKey(0x43)
        foreach ($i in 1..30) {
            Start-Sleep -Milliseconds 100
            $overlay = [CuterInputProbe]::FindWindow([uint32]$script:appPid, $titleShot)
            if ($overlay -ne [IntPtr]::Zero) { break }
        }
        if ($overlay -ne [IntPtr]::Zero) { break }
        if ($attempt -lt 3) { Start-Sleep -Milliseconds 800 }
    }
    if ($overlay -eq [IntPtr]::Zero) { throw (Blocked-Message "the overlay did not appear (cycle $index)") }
    Start-Sleep -Milliseconds 150
    [CuterInputProbe]::Key(0x1B)   # Esc = cancel
    # Wait until the overlay is really gone before the next cycle, otherwise the next
    # Alt+C lands while the previous overlay is still tearing down.
    foreach ($i in 1..40) {
        Start-Sleep -Milliseconds 100
        if ([CuterInputProbe]::FindWindow([uint32]$script:appPid, $titleShot) -eq [IntPtr]::Zero) { return }
    }
    throw 'the overlay never closed'
}

$app = Start-App
$script:appPid = [uint32]$app.Id
# Take the keyboard back from whatever is in front (a running VM window grabs it while
# active, and then synthesized Alt+C / Esc reach nobody).
[void][CuterInputProbe]::FocusDesktop()
Start-Sleep -Milliseconds 500
Say "app pid=$($app.Id); warming up with $Warmup cycles"

foreach ($i in 1..$Warmup) { Invoke-Cycle $i }
Start-Sleep -Seconds 2
$base = Get-Counters $app
Show-Counters 'baseline' $base

$sw = [System.Diagnostics.Stopwatch]::StartNew()
foreach ($i in 1..$Cycles) {
    Invoke-Cycle $i
    if ($i % 20 -eq 0) { Say ("  {0}/{1} cycles ({2:N1}s)" -f $i, $Cycles, $sw.Elapsed.TotalSeconds) }
}
Start-Sleep -Seconds 2
$after = Get-Counters $app
Show-Counters "after $Cycles" $after

# Tolerances: a couple of objects can legitimately appear (fonts, GL caches, the tray
# icon's own objects on first loss/restore). Anything growing with the cycle count is a
# leak, so the limits are deliberately far below "one per capture".
$toleranceGdi = 10
$toleranceUser = 10
$toleranceHandles = 32
$tolerancePrivateMb = 24

$fail = @()
if ($after.Gdi - $base.Gdi -gt $toleranceGdi) { $fail += "GDI objects grew by $($after.Gdi - $base.Gdi) (limit $toleranceGdi)" }
if ($after.User - $base.User -gt $toleranceUser) { $fail += "USER objects grew by $($after.User - $base.User) (limit $toleranceUser)" }
if ($after.Handles - $base.Handles -gt $toleranceHandles) { $fail += "handles grew by $($after.Handles - $base.Handles) (limit $toleranceHandles)" }
if ($after.Private - $base.Private -gt $tolerancePrivateMb) { $fail += "private bytes grew by $([math]::Round($after.Private - $base.Private,1)) MB (limit $tolerancePrivateMb)" }

Say ("delta: GDI {0:+#;-#;0}  USER {1:+#;-#;0}  handles {2:+#;-#;0}  private {3:+#.#;-#.#;0} MB" -f `
    ($after.Gdi - $base.Gdi), ($after.User - $base.User), ($after.Handles - $base.Handles), ($after.Private - $base.Private))
Say ("per cycle: {0:N2}s" -f ($sw.Elapsed.TotalSeconds / $Cycles))

Stop-AllCuter
if (Test-Path $configBackup) { Copy-Item $configBackup $configPath -Force }

if ($fail.Count -gt 0) {
    foreach ($f in $fail) { Write-Host "FAIL: $f" }
    throw 'leak probe failed'
}
Say "RESULT: PASS - $Cycles capture cycles leaked nothing measurable"
