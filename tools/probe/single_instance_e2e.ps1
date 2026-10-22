# cuter single-instance probe.
#
# ASCII ONLY, on purpose: Windows PowerShell 5.1 decodes a BOM-less script as ANSI(GBK),
# and a multi-byte sequence can swallow the following newline (see tools/probe/README.md).
# Chinese strings that must be matched are built at runtime from code points.
#
# Verifies three things with real processes:
#   1. a second `cuter.exe` does NOT stay alive while the first one runs
#   2. it is not silently ignored either - the request really arrives:
#      - a plain second launch makes the RUNNING instance show its settings window
#      - `cuter --capture` makes it open its freeze overlay
#   3. without a running instance, `cuter --capture` still starts a full instance
#      (the guard must not block legitimate first launches)
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\single_instance_e2e.ps1
#
# It temporarily replaces %APPDATA%\Cuter\config.toml and restores it in a finally block.
#
# -Exe <path>  probe another binary (default target\debug\cuter.exe);
#              use it to verify the actual release artifact:
#              ... -File .\tools\probe\single_instance_e2e.ps1 -Exe .\target\release\cuter.exe

param([string]$Exe = '')

$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class SiProbe {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hwnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hwnd, StringBuilder s, int n);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    public static string TitleOf(IntPtr hwnd) {
        var sb = new StringBuilder(512);
        GetWindowTextW(hwnd, sb, 512);
        return sb.ToString();
    }

    public static string ClassOf(IntPtr hwnd) {
        var sb = new StringBuilder(512);
        GetClassNameW(hwnd, sb, 512);
        return sb.ToString();
    }

    /// Visible, titled, non-console top-level windows of a process.
    public static List<IntPtr> TitledWindows(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid || !IsWindowVisible(hwnd)) { return true; }
            if (TitleOf(hwnd).Length == 0) { return true; }
            if (ClassOf(hwnd) == "ConsoleWindowClass") { return true; }
            found.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static IntPtr FindByTitle(uint pid, string title) {
        foreach (var hwnd in TitledWindows(pid)) {
            if (TitleOf(hwnd) == title) { return hwnd; }
        }
        return IntPtr.Zero;
    }

    public static string Describe(IntPtr hwnd) {
        RECT r;
        GetWindowRect(hwnd, out r);
        return string.Format("{0} {1}x{2} title='{3}'", hwnd,
            r.Right - r.Left, r.Bottom - r.Top, TitleOf(hwnd));
    }
}
'@

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\debug\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - run 'cargo build' first" }
Write-Host "probing $((Resolve-Path $exe).Path)"

# Window titles are Chinese; build them here so this file stays pure ASCII.
$overlayTitle = 'Cuter ' + [char]0x622A + [char]0x56FE          # "Cuter " + jie tu
$settingsTitle = 'Cuter'

$config = Join-Path $env:APPDATA 'Cuter\config.toml'
$backup = "$config.single-instance-bak"
$hadConfig = Test-Path $config

$failures = 0
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "PASS: $what" } else { Write-Host "FAIL: $what"; $script:failures++ }
}

function Stop-AllCuter {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 600
}

# Wait for a title to show up on a process; returns the hwnd or [IntPtr]::Zero.
function Wait-Window([int]$pid_, [string]$title, [int]$timeoutMs = 10000) {
    $deadline = (Get-Date).AddMilliseconds($timeoutMs)
    while ((Get-Date) -lt $deadline) {
        $hwnd = [SiProbe]::FindByTitle([uint32]$pid_, $title)
        if ($hwnd -ne [IntPtr]::Zero) { return $hwnd }
        Start-Sleep -Milliseconds 250
    }
    return [IntPtr]::Zero
}

try {
    New-Item -ItemType Directory -Force -Path (Split-Path $config) | Out-Null
    if ($hadConfig) { Copy-Item $config $backup -Force }
    # Start hidden in the tray so no settings window interferes with the assertions.
    Set-Content -Path $config -Value "[general]`nstart_in_tray = true`nclose_to_tray = true`n" -Encoding utf8

    # ---- 1. no instance running: a real instance must start, and stay hidden ----
    Stop-AllCuter
    $first = Start-Process -FilePath $exe -PassThru
    Start-Sleep -Seconds 6
    Check (-not $first.HasExited) 'no instance running: `cuter` starts and keeps running'
    $visible = [SiProbe]::TitledWindows([uint32]$first.Id).Count
    Check ($visible -eq 0) "start_in_tray = true is honoured (visible titled windows = $visible)"

    # ---- 2a. plain second launch: hand the "show the window" request over ----
    #
    # Called with `&`, not Start-Process: PowerShell waits for console-subsystem
    # children and leaves the exit code in $LASTEXITCODE. That is also a live check of
    # the promise "CLI runs are still waited for" (see docs/HANDOFF.md 13.1).
    # Start-Process -PassThru occasionally returns $null for a process that exits
    # immediately, which is exactly what a handover does.
    $againOut = & $exe 2>&1
    $againCode = $LASTEXITCODE
    Check ($againCode -eq 0) "second plain launch exit code = $againCode (expected 0)"
    Check ("$againOut" -like '*Cuter*') "second plain launch printed a handover note to stdout: $againOut"
    $settings = Wait-Window $first.Id $settingsTitle
    Check ($settings -ne [IntPtr]::Zero) "running instance showed its settings window ($([SiProbe]::Describe($settings)))"

    # ---- 2b. `--capture`: hand the capture request over ----
    $captureOut = & $exe --capture 2>&1
    $captureCode = $LASTEXITCODE
    Check ($captureCode -eq 0) "second `--capture` exit code = $captureCode (expected 0)"
    Check (-not $first.HasExited) 'first instance is still alive after the handover'
    $overlay = Wait-Window $first.Id $overlayTitle
    Check ($overlay -ne [IntPtr]::Zero) "running instance opened its overlay ($([SiProbe]::Describe($overlay)))"

    Stop-AllCuter

    # ---- 3. control case: with nobody running, `--capture` starts a full instance ----
    $solo = Start-Process -FilePath $exe -ArgumentList '--capture' -PassThru
    Start-Sleep -Seconds 6
    Check (-not $solo.HasExited) 'with no instance running, `--capture` starts a full instance (guard does not block it)'
} finally {
    Stop-AllCuter
    if ($hadConfig) { Copy-Item $backup $config -Force; Remove-Item $backup -Force }
    Write-Host "config restored from backup: $hadConfig"
}

if ($failures -gt 0) { Write-Host "RESULT: FAIL ($failures)"; exit 1 }
Write-Host 'RESULT: PASS - single instance guard + request handover verified'
exit 0
