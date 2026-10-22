# cuter `general.close_to_tray` end-to-end probe.
#
# Proves BOTH branches of the switch with a real window and a real WM_CLOSE:
#   close_to_tray = true  -> the window disappears, the process KEEPS RUNNING (tray)
#   close_to_tray = false -> the process EXITS
#
# Run with pwsh (7.x). ASCII-only on purpose: no BOM/encoding traps.
#
#   pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\tray_close_e2e.ps1
#
# It temporarily replaces %APPDATA%\Cuter\config.toml and restores it in a finally block.

$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class TrayCloseProbe {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hwnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hwnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);

    public const uint WM_CLOSE = 0x0010;

    /// Visible top-level windows that carry a title AND are not the console window.
    ///
    /// The console filter is load-bearing: cuter.exe is a console-subsystem binary,
    /// so `Start-Process` gives it a visible `ConsoleWindowClass` window whose title is
    /// the exe path. Posting WM_CLOSE there closes the console (and kills the attached
    /// process a few seconds later) instead of exercising the app's own close handling -
    /// which looks exactly like "close_to_tray is broken".
    public static List<IntPtr> TitledWindows(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid || !IsWindowVisible(hwnd)) { return true; }
            var title = new StringBuilder(256);
            if (GetWindowTextW(hwnd, title, 256) <= 0) { return true; }
            var cls = new StringBuilder(256);
            GetClassNameW(hwnd, cls, 256);
            if (cls.ToString() == "ConsoleWindowClass") { return true; }
            found.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)   # repo root
$exe = Join-Path $root 'target\debug\cuter.exe'
if (-not (Test-Path $exe)) { throw "missing $exe - run 'cargo build' first" }

$config = Join-Path $env:APPDATA 'Cuter\config.toml'
$backup = "$config.tray-close-bak"
$hadConfig = Test-Path $config

function Stop-AllCuter {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 400
}

function Start-Cuter([bool]$closeToTray) {
    $body = "[general]`nstart_in_tray = false`nclose_to_tray = $($closeToTray.ToString().ToLower())`n"
    Set-Content -Path $config -Value $body -Encoding utf8
    Stop-AllCuter
    return Start-Process -FilePath $exe -PassThru
}

function Wait-TitledWindow($process, [int]$timeoutMs = 20000) {
    $deadline = (Get-Date).AddMilliseconds($timeoutMs)
    while ((Get-Date) -lt $deadline) {
        $windows = [TrayCloseProbe]::TitledWindows([uint32]$process.Id)
        if ($windows.Count -gt 0) { return $windows[0] }
        Start-Sleep -Milliseconds 250
    }
    return [IntPtr]::Zero
}

$failures = 0
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "PASS: $what" } else { Write-Host "FAIL: $what"; $script:failures++ }
}

try {
    New-Item -ItemType Directory -Force -Path (Split-Path $config) | Out-Null
    if ($hadConfig) { Copy-Item $config $backup -Force }

    # ---- branch 1: close_to_tray = true -> hide, keep running ----
    $p = Start-Cuter $true
    $hwnd = Wait-TitledWindow $p
    Check ($hwnd -ne [IntPtr]::Zero) "close_to_tray=true: main window appeared (hwnd=$hwnd)"
    if ($hwnd -ne [IntPtr]::Zero) {
        [void][TrayCloseProbe]::PostMessage($hwnd, [TrayCloseProbe]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Seconds 3
        $alive = -not $p.HasExited
        $visible = [TrayCloseProbe]::TitledWindows([uint32]$p.Id).Count
        Check $alive 'close_to_tray=true: process is STILL RUNNING after WM_CLOSE (tray mode)'
        Check ($visible -eq 0) "close_to_tray=true: window is hidden (visible titled windows = $visible)"
    }
    Stop-AllCuter

    # ---- branch 2: close_to_tray = false -> exit ----
    $p2 = Start-Cuter $false
    $hwnd2 = Wait-TitledWindow $p2
    Check ($hwnd2 -ne [IntPtr]::Zero) 'close_to_tray=false: main window appeared'
    if ($hwnd2 -ne [IntPtr]::Zero) {
        [void][TrayCloseProbe]::PostMessage($hwnd2, [TrayCloseProbe]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        $exited = $p2.WaitForExit(10000)
        Check $exited 'close_to_tray=false: process EXITED after WM_CLOSE'
    }
} finally {
    Stop-AllCuter
    if ($hadConfig) { Copy-Item $backup $config -Force; Remove-Item $backup -Force }
    Write-Host "config restored from backup: $hadConfig"
}

if ($failures -gt 0) { Write-Host "RESULT: FAIL ($failures)"; exit 1 }
Write-Host 'RESULT: PASS - both branches of general.close_to_tray verified'
exit 0
