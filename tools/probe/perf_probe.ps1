# cuter performance probe: memory in real states + end-to-end latencies.
#
# ASCII only (see tools/probe/README.md).
#
# Why this exists: the project's pitch is "millisecond startup, ~7 MB on disk, small
# memory". Those are claims that MUST be numbers, not adjectives - and numbers drift.
# Run this before a release and paste the table into the commit / README.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\perf_probe.ps1
#   ... -Exe .\target\release\cuter.exe -SettleSeconds 6
#
# States measured:
#   tray (hidden window, the steady state) / settings window / freeze overlay
#   plus: --full end-to-end, and cold start -> freeze overlay visible
#
# It kills every cuter process it starts (including any instance left over from
# earlier runs - a leftover instance would steal the global hotkeys and answer
# single-instance handovers, which would silently corrupt these numbers).

param(
    [string]$Exe = '',
    [int]$SettleSeconds = 6
)

$ErrorActionPreference = 'Stop'

# --full follows the user's config, so read which backend this run actually measures.
# Printing the number without it is what made an earlier comparison confusing: GDI and
# DXGI were tens of milliseconds apart and the logs did not say which one was measured.
$captureBackend = 'gdi (app default)'
$configPath = Join-Path $env:APPDATA 'Cuter\config.toml'
if (Test-Path $configPath) {
    $backendLine = Select-String -Path $configPath -Pattern '^\s*backend\s*=\s*"(\w+)"' | Select-Object -First 1
    if ($backendLine) { $captureBackend = $backendLine.Matches[0].Groups[1].Value }
}

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class PerfProbe {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hwnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hwnd, StringBuilder s, int n);

    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT { public uint type; public INPUTUNION u; }
    // The union MUST contain the largest member (`MOUSEINPUT`), otherwise
    // `Marshal.SizeOf(INPUT)` is 32 instead of the 40 that x64 `SendInput`
    // demands, the call fails with ERROR_INVALID_PARAMETER, and the key is
    // silently never injected (measured: every run then "FAIL"s).
    [StructLayout(LayoutKind.Explicit)]
    public struct INPUTUNION {
        [FieldOffset(0)] public MOUSEINPUT mi;
        [FieldOffset(0)] public KEYBDINPUT ki;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)]
    public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);

    const uint INPUT_KEYBOARD = 1;
    const uint KEYEVENTF_KEYUP = 0x0002;
    const ushort VK_MENU = 0x12;

    /// Size that `SendInput` requires for one `INPUT` (40 on x64, 28 on x86).
    public static int InputSize() { return Marshal.SizeOf(typeof(INPUT)); }

    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    public const uint WM_CLOSE = 0x0010;

    /// Find a window by pid + a title fragment.
    /// Closing the settings window MUST NOT rely on focus (Esc does), otherwise a probe can
    /// report "parked" while the window is still open and the memory number is a lie.
    public static IntPtr FindWindowByPid(uint pid, string titlePart) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid) return true;
            var sb = new StringBuilder(256); GetWindowTextW(hwnd, sb, 256);
            if (sb.ToString().Contains(titlePart)) { found = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static bool PostClose(IntPtr hwnd) { return PostMessage(hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero); }

    /// Press Alt+<vk> as ONE atomic SendInput batch, with zero sleeps.
    /// Returns how many events were inserted (must be 4).
    ///
    /// This matters for the latency number: the older `keybd_event` chord helper
    /// sleeps 50+50+30+400 ms *inside* the call, so a stopwatch wrapped around it
    /// measures the script's own sleeps (>=530 ms), not the application.
    public static uint AltChord(byte vk) {
        var inputs = new INPUT[4];
        inputs[0].type = INPUT_KEYBOARD; inputs[0].u.ki.wVk = VK_MENU;
        inputs[1].type = INPUT_KEYBOARD; inputs[1].u.ki.wVk = vk;
        inputs[2].type = INPUT_KEYBOARD; inputs[2].u.ki.wVk = vk;     inputs[2].u.ki.dwFlags = KEYEVENTF_KEYUP;
        inputs[3].type = INPUT_KEYBOARD; inputs[3].u.ki.wVk = VK_MENU; inputs[3].u.ki.dwFlags = KEYEVENTF_KEYUP;
        return SendInput(4, inputs, Marshal.SizeOf(typeof(INPUT)));
    }

    /// Inject Alt+<vk> and poll (spin, no `Start-Sleep`) until a visible window of
    /// `pid` titled exactly `title` exists.
    /// Returns elapsed ms, -1 on timeout, or -2 if the injection itself failed.
    public static int MeasureOverlayLatency(uint pid, string title, byte vk, int timeoutMs) {
        var sw = System.Diagnostics.Stopwatch.StartNew();
        if (AltChord(vk) != 4) { return -2; }
        while (sw.ElapsedMilliseconds < timeoutMs) {
            if (HasTitle(pid, title)) { sw.Stop(); return (int)sw.ElapsedMilliseconds; }
            System.Threading.Thread.SpinWait(300);
        }
        return -1;
    }

    public static string TitleOf(IntPtr hwnd) {
        var sb = new StringBuilder(512);
        GetWindowTextW(hwnd, sb, 512);
        return sb.ToString();
    }

    /// First visible, titled, non-console window of `pid` whose title equals `title`.
    public static bool HasTitle(uint pid, string title) {
        bool found = false;
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid || !IsWindowVisible(hwnd)) { return true; }
            if (TitleOf(hwnd) != title) { return true; }
            var cls = new StringBuilder(256);
            GetClassNameW(hwnd, cls, 256);
            if (cls.ToString() == "ConsoleWindowClass") { return true; }
            found = true;
            return false;
        }, IntPtr.Zero);
        return found;
    }
}
'@

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$exe = if ($Exe) { $Exe } else { Join-Path $root 'target\release\cuter.exe' }
if (-not (Test-Path $exe)) { throw "missing $exe - build it first" }
$exe = (Resolve-Path $exe).Path
Write-Host "probing $exe"

# "Cuter " + jie tu, built here so this file stays pure ASCII.
$overlayTitle = 'Cuter ' + [char]0x622A + [char]0x56FE

function Get-Stat($proc) {
    $proc.Refresh()
    [PSCustomObject]@{
        WorkingSetMB = [math]::Round($proc.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
        Handles      = $proc.HandleCount
        Threads      = $proc.Threads.Count
    }
}

function Format-Stat([string]$label, $stat) {
    "{0,-30} WS={1,7:N1} MB  Private={2,7:N1} MB  Handles={3,4}  Threads={4}" -f `
        $label, $stat.WorkingSetMB, $stat.PrivateMB, $stat.Handles, $stat.Threads
}

function Stop-AllCuter {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

Stop-AllCuter

# Self-check before measuring anything: if the `INPUT` struct has the wrong size,
# `SendInput` fails with ERROR_INVALID_PARAMETER and the key is silently never
# injected - every hotkey run then reports "FAIL", which looks like an app bug.
$expectedInputSize = if ([IntPtr]::Size -eq 8) { 40 } else { 28 }
if ([PerfProbe]::InputSize() -ne $expectedInputSize) {
    throw "INPUT size is $([PerfProbe]::InputSize()), expected $expectedInputSize - SendInput would do nothing"
}

Write-Host ''
Write-Host '--- hotkey -> overlay latency, clean tray instance (no window of ours on screen) ---'
# stderr is captured so the internal stage timings ("截图链路[...]：+N ms") end up in
# the same report as the wall-clock latency - that is how you tell WHERE the time goes.
$trayLog = Join-Path $root 'target\smoke\perf-tray.log'
$tray = Start-Process -FilePath $exe -PassThru -RedirectStandardError $trayLog
Start-Sleep -Seconds 3

Add-Type -Path (Join-Path $PSScriptRoot 'CuterInputProbe.cs')
function Wait-Overlay([bool]$want, [int]$timeoutMs = 10000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $seen = [PerfProbe]::HasTitle([uint32]$tray.Id, $overlayTitle)
        if ($seen -eq $want) { $sw.Stop(); return $sw.ElapsedMilliseconds }
        Start-Sleep -Milliseconds 5
    }
    return -1
}

# This is the number the roadmap's "hotkey -> mask" target is about: the instance sits
# in the tray with every window hidden, exactly like it does all day.
# 9 samples, not 5: this machine's spread for the SAME binary has been observed at
# 106 vs 209 ms, which is larger than any difference we would want to detect. Five
# samples cannot separate signal from that noise.
$hotkeyTimes = @()
foreach ($i in 1..9) {
    $gone = Wait-Overlay $false 3000
    if ($gone -lt 0) {
        [void][CuterInputProbe]::Activate([int]$tray.MainWindowHandle)
        [CuterInputProbe]::Key(0x1B)
        $gone = Wait-Overlay $false 5000
        if ($gone -lt 0) { Write-Host "  run ${i}: could not dismiss the overlay, skipping"; continue }
    }
    Start-Sleep -Milliseconds 400
    # One atomic Alt+C (no sleeps inside the injection) + a spinning poll: the
    # number below is the application's latency, not this script's timer jitter.
    $ms = [PerfProbe]::MeasureOverlayLatency([uint32]$tray.Id, $overlayTitle, 0x43, 10000)
    if ($ms -ge 0) { $hotkeyTimes += $ms; Write-Host ("  run {0}: {1} ms" -f $i, $ms) }
    elseif ($ms -eq -2) { Write-Host "  run ${i}: INJECT FAILED (SendInput rejected the batch)" }
    else { Write-Host "  run ${i}: FAIL (no overlay within 10 s)" }
}
if ($hotkeyTimes.Count -gt 0) {
    $sorted = @($hotkeyTimes | Sort-Object)
    $median = $sorted[[int][math]::Floor($sorted.Count / 2)]
    $p25 = $sorted[[int][math]::Floor($sorted.Count * 0.25)]
    $p75 = $sorted[[int][math]::Floor($sorted.Count * 0.75)]
    Write-Host ("Alt+C -> overlay visible (tray, nothing of ours on screen): min {0} / p25 {1} / median {2} / p75 {3} / max {4} ms ({5} runs)" -f `
        $sorted[0], $p25, $median, $p75, $sorted[-1], $sorted.Count)
    if ($median -gt 0) {
        $spread = [math]::Round(($sorted[-1] - $sorted[0]) / $median, 2)
        Write-Host ("  spread: {0}x of the median between the fastest and slowest run - do not read a single run as a result" -f $spread)
    }
}
[void][CuterInputProbe]::Activate([int]$tray.MainWindowHandle)
[CuterInputProbe]::Key(0x1B)
Start-Sleep -Milliseconds 500

Write-Host ''
Write-Host '--- tray mode memory (settings window open = steady state) ---'
# Force the lazy one-time work BEFORE sampling, or the numbers are not comparable
# between runs: the CJK font (~19 MB file) is parsed on the first text layout, which
# costs tens of MB, and in tray mode nothing lays out text until a window is shown.
# `cuter` (no args) hands a "show the settings window" request to this instance.
& $exe | Out-Null
Start-Sleep -Seconds $SettleSeconds
$stat = Get-Stat $tray
Write-Host (Format-Stat "tray after $SettleSeconds s" $stat)

# Idle growth is the "does it leak while sitting in the tray" check.
$before = $stat
Start-Sleep -Seconds $SettleSeconds
$after = Get-Stat $tray
Write-Host (Format-Stat "tray after $($SettleSeconds * 2) s" $after)
Write-Host ("idle growth over {0}s: {1} MB" -f $SettleSeconds,
    [math]::Round($after.WorkingSetMB - $before.WorkingSetMB, 1))
Write-Host (Format-Stat 'settings window open' (Get-Stat $tray))

Write-Host ''
Write-Host '--- `--capture` from a second process (this one DOES include its own startup) ---'
$sw = [System.Diagnostics.Stopwatch]::StartNew()
& $exe --capture | Out-Null
$seen = $false
while (-not $seen -and $sw.ElapsedMilliseconds -lt 10000) {
    $seen = [PerfProbe]::HasTitle([uint32]$tray.Id, $overlayTitle)
    if (-not $seen) { Start-Sleep -Milliseconds 5 }
}
$sw.Stop()
if ($seen) {
    Write-Host ("overlay visible after {0} ms (settings window WAS on screen: needs the hide delay)" -f $sw.ElapsedMilliseconds)
} else {
    Write-Host 'FAIL: warm overlay never appeared within 10 s'
}
Start-Sleep -Seconds 2
# Heaviest steady state: a full-desktop frozen frame (1920x1080 RGBA = 8.3 MB)
# plus its egui texture, plus the toolbar.
Write-Host (Format-Stat 'freeze overlay open' (Get-Stat $tray))
Stop-AllCuter

Write-Host ''
Write-Host '--- where the time goes (app stderr, last 8 stage lines) ---'
if (Test-Path $trayLog) {
    # Stage lines look like: "<ts> INFO cuter_app::app] <stage>：+123 ms" - matched on
    # the ASCII tail so this file can stay pure ASCII. The log itself is UTF-8, so it
    # must be read as UTF-8 or the Chinese stage names come out as mojibake.
    $stages = Select-String -Path $trayLog -Pattern ' ms$' -Encoding UTF8 | Select-Object -Last 8
    if ($stages) { $stages | ForEach-Object { Write-Host "  $($_.Line)" } }
    else { Write-Host '  (no stage lines captured)' }
} else {
    Write-Host "  (no log at $trayLog)"
}
Stop-AllCuter

Write-Host ''
Write-Host ('--- headless full-screen capture, end to end (3 runs, backend={0}) ---' -f $captureBackend)
$out = Join-Path $root 'target\smoke\perf-full.png'
$times = @()
foreach ($i in 1..3) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $exe --full --out $out | Out-Null
    $sw.Stop()
    $times += $sw.ElapsedMilliseconds
    Write-Host ("  run {0}: {1} ms" -f $i, $sw.ElapsedMilliseconds)
}
$sorted = $times | Sort-Object
Write-Host ("--full (process start -> PNG on disk, backend={4}): min {0} ms, median {1} ms, max {2} ms, {3:N0} bytes" -f `
    $sorted[0], $sorted[1], $sorted[2], (Get-Item $out).Length, $captureBackend)

Write-Host ''
Write-Host '--- cold start -> freeze overlay visible ---'
# This is the number a user feels when they press the hotkey on a cold process:
# process start + full-desktop GDI capture + overlay window creation.
$cold = Start-Process -FilePath $exe -ArgumentList '--capture' -PassThru
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$seen = $false
while (-not $seen -and $sw.ElapsedMilliseconds -lt 20000) {
    $seen = [PerfProbe]::HasTitle([uint32]$cold.Id, $overlayTitle)
    if (-not $seen) { Start-Sleep -Milliseconds 5 }
}
$sw.Stop()
if ($seen) {
    Write-Host ("overlay visible after {0} ms (cold process)" -f $sw.ElapsedMilliseconds)
} else {
    Write-Host 'FAIL: overlay never appeared within 20 s'
}

Stop-AllCuter
Write-Host ''
Write-Host 'done (killed every cuter process it started)'
