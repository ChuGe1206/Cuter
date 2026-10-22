# The overlay must own its very top pixel row.
#
# ASCII only (see tools/probe/README.md).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\probe\overlay_top_edge_e2e.ps1
#
# Why this exists (user report): with the pointer on the TOP edge of the screen,
# pressing the left button and dragging moved the whole frozen frame around. The
# cause was not our drag handling at all - winit's undecorated window still carries
# WS_CAPTION, and for a window that fills the screen DefWindowProc answers
# WM_NCHITTEST with HTCAPTION exactly on the topmost row. The OS then runs its own
# "drag the caption" move loop and our process never sees the press.
#
# So the check is two-part:
#   A. WM_NCHITTEST on the top row (centre and both corners) answers HTCLIENT
#   B. a drag started exactly on that row leaves the window rect untouched
#
# Exit codes: 0 pass, 1 fail, 2 skip (no overlay / window is not screen-sized).
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
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CutTopEdgeHit {
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
}
'@

function Shot([string]$path) {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose()
    $bmp.Dispose()
}

function RectOf($h) {
    $r = [CuterInputProbe]::WindowRect($h)
    return ("{0},{1} {2}x{3}" -f $r.Left, $r.Top, ($r.Right - $r.Left), ($r.Bottom - $r.Top))
}

Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
$proc = Start-Process -FilePath $exe -PassThru
Start-Sleep -Milliseconds 4200
[void][CuterInputProbe]::SetPhysicalDpiAwareness()

# Alt+C is the default region-capture hotkey.
[CuterInputProbe]::AltKey(0x43)

$ov = [IntPtr]::Zero
for ($i = 0; $i -lt 90; $i++) {
    $script:found = [IntPtr]::Zero
    [CuterInputProbe]::EnumWindows({
        param($h, $l)
        $p = 0
        [CuterInputProbe]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
        if ($p -eq $proc.Id -and [CuterInputProbe]::IsWindowVisible($h)) {
            $r = [CuterInputProbe]::WindowRect($h)
            if (($r.Right - $r.Left) -ge ($screen.Width - 8)) { $script:found = $h }
        }
        return $true
    }, [IntPtr]::Zero) | Out-Null
    if ($script:found -ne [IntPtr]::Zero) { $ov = $script:found; break }
    Start-Sleep -Milliseconds 100
}

if ($ov -eq [IntPtr]::Zero) {
    Say 'SKIP: the overlay window never showed up'
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    exit 2
}
Start-Sleep -Milliseconds 900

$rect = RectOf $ov
Say "overlay rect: $rect"
if ($rect -ne ("{0},{1} {2}x{3}" -f $screen.Left, $screen.Top, $screen.Width, $screen.Height)) {
    Say 'SKIP: the overlay is not exactly screen-sized, hit-test band would be elsewhere'
    [CuterInputProbe]::Key(0x1B)
    Start-Sleep -Milliseconds 300
    Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force
    exit 2
}

$fail = 0
$names = @{ 1 = 'HTCLIENT'; 2 = 'HTCAPTION'; 10 = 'HTLEFT'; 11 = 'HTRIGHT'; 12 = 'HTTOP'; 13 = 'HTTOPLEFT'; 14 = 'HTTOPRIGHT' }
foreach ($pt in @(@(($screen.Width / 2)), @(2), @(($screen.Width - 2)))) {
    $x = [int]$pt[0]
    $y = $screen.Top
    $lp = [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
    $code = [CutTopEdgeHit]::SendMessage($ov, 0x0084, [IntPtr]::Zero, $lp).ToInt32()
    $nm = if ($names.ContainsKey($code)) { $names[$code] } else { "other($code)" }
    Say ("hit-test ({0},{1}) = {2}" -f $x, $y, $nm)
    if ($code -ne 1) { $fail = 1 }
}

$before = RectOf $ov
$tempRoot = [System.IO.Path]::GetTempPath()
$beforeShot = Join-Path $tempRoot 'cuter-top-edge-before.png'
$duringShot = Join-Path $tempRoot 'cuter-top-edge-during.png'
Shot $beforeShot

# Press exactly on the top row, then drag down-right. Before the fix the OS moved
# the window here; after it, this is just an ordinary selection drag.
[CuterInputProbe]::MoveTo(($screen.Width / 2), $screen.Top) | Out-Null
Start-Sleep -Milliseconds 400
[CuterInputProbe]::LeftDown()
Start-Sleep -Milliseconds 150
for ($s = 1; $s -le 6; $s++) {
    [CuterInputProbe]::MoveTo((($screen.Width / 2) - 20 * $s), ($screen.Top + 24 * $s)) | Out-Null
    Start-Sleep -Milliseconds 80
}
Start-Sleep -Milliseconds 200
$during = RectOf $ov
Shot $duringShot
[CuterInputProbe]::LeftUp()
Start-Sleep -Milliseconds 400
$after = RectOf $ov

Say "rect before:   $before"
Say "rect dragging: $during"
Say "rect after:    $after"
if ($during -ne $before -or $after -ne $before) {
    Say 'FAIL: dragging from the top edge moved the overlay window'
    $fail = 1
}

[CuterInputProbe]::Key(0x1B)
Start-Sleep -Milliseconds 300
Get-Process cuter -ErrorAction SilentlyContinue | Stop-Process -Force

if ($fail -eq 0) {
    Say 'RESULT: PASS'
    exit 0
}
Say 'RESULT: FAIL'
exit 1
