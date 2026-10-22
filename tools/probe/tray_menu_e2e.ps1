param(
    [string]$Exe = '',
    # TEMP by default so that run_all.ps1 cannot dirty the archived evidence shots;
    # to refresh the archived one, pass -ShotDir docs\design\shots -ShotName 0.1.48-tray-menu.png.
    [string]$ShotDir = '',
    [string]$ShotName = 'tray-menu.png'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if ([string]::IsNullOrEmpty($Exe)) { $Exe = Join-Path $root 'target\release\cuter.exe' }
if ([string]::IsNullOrEmpty($ShotDir)) { $ShotDir = $env:TEMP }
$failures = New-Object System.Collections.Generic.List[string]
$skips = New-Object System.Collections.Generic.List[string]

function Fail([string]$message) {
    $script:failures.Add($message) | Out-Null
    Write-Host "FAIL $message"
}

function Skip([string]$message) {
    $script:skips.Add($message) | Out-Null
    Write-Host "skip $message"
}

function Ok([string]$message) {
    Write-Host "ok   $message"
}

Add-Type -Path (Join-Path $root 'tools\probe\CuterInputProbe.cs')

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class TrayRightClick {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hwnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct NOTIFYICONIDENTIFIER {
        public uint cbSize;
        public IntPtr hWnd;
        public uint uID;
        public Guid guidItem;
    }

    [DllImport("shell32.dll")]
    public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT rect);

    /// The screen rect of the tray icon registered by (hwnd, uid), or null when the
    /// shell does not have it on screen (for example while it hides in the overflow).
    public static string IconRect(IntPtr hwnd, uint uid) {
        var id = new NOTIFYICONIDENTIFIER {
            cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER)),
            hWnd = hwnd,
            uID = uid,
            guidItem = Guid.Empty,
        };
        RECT rect;
        int hr = Shell_NotifyIconGetRect(ref id, out rect);
        if (hr != 0) { return null; }
        return string.Format("{0},{1} {2}x{3}", rect.Left, rect.Top, rect.Right - rect.Left, rect.Bottom - rect.Top);
    }

    public static string[] ProcessWindows(uint pid) {
        var lines = new List<string>();
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid) { return true; }
            var cls = new StringBuilder(256);
            GetClassNameW(hwnd, cls, 256);
            lines.Add(string.Format("{0}|{1}|visible={2}", hwnd, cls, IsWindowVisible(hwnd)));
            return true;
        }, IntPtr.Zero);
        return lines.ToArray();
    }

    public static IntPtr ChildByClass(IntPtr parent, string cls, bool visibleOnly) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, (hwnd, _) => {
            var b = new StringBuilder(256);
            GetClassNameW(hwnd, b, 256);
            if (b.ToString() == cls && (!visibleOnly || IsWindowVisible(hwnd))) { found = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    /// "<hwnd>|<left>,<top> <w>x<h>" of the taskbar button that opens the hidden-icon
    /// overflow, or an empty string when the taskbar has no such button.
    public static string Chevron() {
        var tray = FindWindowW("Shell_TrayWnd", null);
        if (tray == IntPtr.Zero) { return ""; }
        var notify = ChildByClass(tray, "TrayNotifyWnd", true);
        if (notify == IntPtr.Zero) { return ""; }
        string text = "";
        EnumChildWindows(notify, (hwnd, _) => {
            var b = new StringBuilder(256);
            GetClassNameW(hwnd, b, 256);
            if (b.ToString() == "Button" && IsWindowVisible(hwnd)) {
                text = string.Format("{0}|{1}", hwnd, RectText(hwnd));
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return text;
    }

    public static string RectText(IntPtr hwnd) {
        RECT r;
        GetWindowRect(hwnd, out r);
        return string.Format("{0},{1} {2}x{3}", r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top);
    }
}
'@

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
[CuterInputProbe]::SetPhysicalDpiAwareness()

function Find-TrayIconRect([uint32]$processId) {
    foreach ($line in [TrayRightClick]::ProcessWindows($processId)) {
        $hwndValue = [int64]($line.Split('|')[0])
        $hwnd = [IntPtr]::new($hwndValue)
        foreach ($uid in 0..16) {
            $rect = [TrayRightClick]::IconRect($hwnd, [uint32]$uid)
            if ($null -ne $rect) {
                Write-Host "tray icon: window=$hwndValue uid=$uid rect=$rect"
                return $rect
            }
        }
    }
    return $null
}

function Test-TrayWindow([uint32]$processId) {
    foreach ($line in [TrayRightClick]::ProcessWindows($processId)) {
        if ($line -match '\|tray_icon_app\|') { return $true }
    }
    return $false
}

function Open-Overflow {
    $chevron = [TrayRightClick]::Chevron()
    if ([string]::IsNullOrEmpty($chevron)) {
        # Classic taskbar layouts and locked-down shells do not have the Win11 chevron.
        Skip 'the taskbar has no overflow chevron, so the icon cannot be reached'
        return $false
    }
    Write-Host "overflow chevron: $chevron"
    $parts = $chevron.Split('|')[1] -split '[ ,x]'
    $x = [int]$parts[0]; $y = [int]$parts[1]; $w = [int]$parts[2]; $h = [int]$parts[3]
    [CuterInputProbe]::MoveTo([int]($x + $w / 2), [int]($y + $h / 2))
    Start-Sleep -Milliseconds 150
    [CuterInputProbe]::LeftDown()
    Start-Sleep -Milliseconds 60
    [CuterInputProbe]::LeftUp()
    Start-Sleep -Milliseconds 1500
    return $true
}

function Take-Shot([string]$path, [string]$rectText) {
    $parts = $rectText -split '[ ,x]'
    $left = [int]$parts[0]; $top = [int]$parts[1]; $width = [int]$parts[2]; $height = [int]$parts[3]
    $pad = 24
    $x = [Math]::Max(0, $left - $pad)
    $y = [Math]::Max(0, $top - $pad)
    $w = [Math]::Min($width + $pad * 2, 1920 - $x)
    $h = [Math]::Min($height + $pad * 2, 1080 - $y)
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose()
    $bmp.Dispose()
    Write-Host "screenshot: $path"
}

$config = Join-Path $env:APPDATA 'Cuter\config.toml'
$backup = "$config.tray-probe-backup"
$hadConfig = Test-Path $config
if ($hadConfig) { Copy-Item $config $backup -Force }
Set-Content -Path $config -Value "[general]`nstart_in_tray = true`ntheme = `"dark`"`n" -Encoding utf8

Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

try {
    $proc = Start-Process -FilePath $Exe -PassThru
    Start-Sleep -Seconds 6
    if ($proc.HasExited) { Fail 'cuter exited during startup'; return }

    $iconRect = Find-TrayIconRect ([uint32]$proc.Id)
    if ($null -eq $iconRect) {
        Write-Host 'the icon is not on screen; opening the overflow'
        if (-not (Open-Overflow)) { return }
        $iconRect = Find-TrayIconRect ([uint32]$proc.Id)
    }
    if ($null -eq $iconRect) {
        if (Test-TrayWindow ([uint32]$proc.Id)) {
            # The icon is registered (the tray message window exists) but the shell will not
            # tell us where it is: that is the shell's business, not a regression of ours.
            Skip 'the shell did not report a rect for our tray icon'
            return
        }
        Fail 'the process has no tray_icon_app window: no tray icon was registered'
        return
    }
    Ok "tray icon is on screen at $iconRect"

    $parts = $iconRect -split '[ ,x]'
    $cx = [int]([int]$parts[0] + [int]$parts[2] / 2)
    $cy = [int]([int]$parts[1] + [int]$parts[3] / 2)
    [CuterInputProbe]::MoveTo($cx, $cy)
    Start-Sleep -Milliseconds 250
    [CuterInputProbe]::RightClick()
    Write-Host "right-clicked the tray icon at $cx,$cy"

    $panel = [IntPtr]::Zero
    for ($i = 0; $i -lt 20; $i++) {
        Start-Sleep -Milliseconds 150
        $panel = [CuterInputProbe]::FindWindow([uint32]$proc.Id, 'Cuter')
        if ($panel -ne [IntPtr]::Zero) { break }
    }
    if ($panel -eq [IntPtr]::Zero) {
        Fail 'the tray menu window did not appear after the right click'
    }
    else {
        Ok "tray menu window appeared: $panel $([CuterInputProbe]::Describe($panel))"
        if (-not [CuterInputProbe]::IsOnDesktop($panel)) { Fail 'the tray menu window is not on the desktop' }
        # The window is created hidden and then asked to show, so the first frames still
        # carry the show animation; wait for it to settle before taking the evidence shot.
        Start-Sleep -Milliseconds 1200
        $panelRect = [TrayRightClick]::RectText($panel)
        $shotPath = Join-Path $ShotDir $ShotName
        Take-Shot $shotPath $panelRect

        [CuterInputProbe]::Key(0x1B)
        $closed = $false
        for ($i = 0; $i -lt 20; $i++) {
            Start-Sleep -Milliseconds 150
            if ([CuterInputProbe]::CountWindows([uint32]$proc.Id, 'Cuter') -eq 0) { $closed = $true; break }
        }
        if ($closed) { Ok 'Escape closed the tray menu' } else { Fail 'Escape did not close the tray menu' }
    }
}
finally {
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    if ($hadConfig) { Copy-Item $backup $config -Force; Remove-Item $backup -Force }
    else { Remove-Item $config -Force -ErrorAction SilentlyContinue }
}

if ($failures.Count -gt 0) {
    Write-Host ''
    Write-Host "RESULT: FAILED ($($failures.Count))"
    $failures | ForEach-Object { Write-Host "  - $_" }
    exit 1
}
if ($skips.Count -gt 0) {
    Write-Host ''
    Write-Host "RESULT: SKIPPED ($($skips.Count))"
    $skips | ForEach-Object { Write-Host "  - $_" }
    exit 2
}
Write-Host ''
Write-Host 'RESULT: PASS'
exit 0
