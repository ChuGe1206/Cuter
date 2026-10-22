// 一个"有控件的窗口"：给控件级吸附探针当靶子。
//
// 里面放一个**位置尺寸完全已知**的原生 BUTTON 子控件（UI Automation 会把它报成
// Button 元素），探针据此断言"点一下正好选中这个按钮"。
// 只用于 tools/probe，不进产物。

using System;
using System.Runtime.InteropServices;
using System.Threading;

public static class ControlWindow
{
    private const int WM_CLOSE = 0x0010;
    private const int WM_DESTROY = 0x0002;
    private const int WS_OVERLAPPEDWINDOW = 0x00CF0000;
    private const int WS_CHILD = 0x40000000;
    private const int WS_VISIBLE = 0x10000000;
    private const int WS_TABSTOP = 0x00010000;
    private const int WS_BORDER = 0x00800000;
    private const int BS_PUSHBUTTON = 0;

    private delegate IntPtr WndProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WNDCLASS
    {
        public uint style;
        public WndProc lpfnWndProc;
        public int cbClsExtra;
        public int cbWndExtra;
        public IntPtr hInstance;
        public IntPtr hIcon;
        public IntPtr hCursor;
        public IntPtr hbrBackground;
        public string lpszMenuName;
        public string lpszClassName;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MSG
    {
        public IntPtr hwnd;
        public uint message;
        public IntPtr wParam;
        public IntPtr lParam;
        public uint time;
        public int ptX;
        public int ptY;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT { public int x; public int y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int left, top, right, bottom; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern ushort RegisterClassW(ref WNDCLASS cls);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr CreateWindowExW(uint exStyle, string className, string windowName, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32.dll")] private static extern IntPtr DefWindowProcW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] private static extern bool GetMessageW(out MSG msg, IntPtr hWnd, uint min, uint max);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref MSG msg);
    [DllImport("user32.dll")] private static extern IntPtr DispatchMessageW(ref MSG msg);
    [DllImport("user32.dll")] private static extern void PostMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] private static extern bool DestroyWindow(IntPtr hWnd);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr hWnd, int cmd);
    [DllImport("user32.dll")] private static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("kernel32.dll")] private static extern IntPtr GetModuleHandleW(string name);
    [DllImport("user32.dll")] private static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] private static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr hWnd, int attr, out RECT value, int size);

    // 按钮在客户区里的位置与尺寸（探针要用它算期望值）
    public const int ButtonX = 40;
    public const int ButtonY = 60;
    public const int ButtonW = 220;
    public const int ButtonH = 64;

    private static IntPtr _hwnd = IntPtr.Zero;
    private static IntPtr _button = IntPtr.Zero;
    private static WndProc _proc;

    public static IntPtr Start(string title, int x, int y, int w, int h)
    {
        var ready = new ManualResetEventSlim(false);
        var thread = new Thread(() =>
        {
            _proc = HandleMessage;
            var cls = new WNDCLASS
            {
                lpfnWndProc = _proc,
                hInstance = GetModuleHandleW(null),
                lpszClassName = "CuterProbeControlWindow",
            };
            RegisterClassW(ref cls);
            _hwnd = CreateWindowExW(0, "CuterProbeControlWindow", title, WS_OVERLAPPEDWINDOW,
                x, y, w, h, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            // 原生 BUTTON：UIA 会报成 Button 控件，矩形就是这里给的
            _button = CreateWindowExW(0, "BUTTON", "Target Button",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                ButtonX, ButtonY, ButtonW, ButtonH, _hwnd, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            ShowWindow(_hwnd, 5);
            SetWindowPos(_hwnd, new IntPtr(-1), x, y, w, h, 0x0040);
            ready.Set();
            MSG msg;
            while (GetMessageW(out msg, IntPtr.Zero, 0, 0))
            {
                TranslateMessage(ref msg);
                DispatchMessageW(ref msg);
            }
        });
        thread.IsBackground = true;
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        ready.Wait(5000);
        return _hwnd;
    }

    public static void Stop()
    {
        if (_hwnd != IntPtr.Zero)
        {
            PostMessageW(_hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
            _hwnd = IntPtr.Zero;
        }
    }

    /// <summary>按钮在屏幕上的**真实**矩形（物理像素）| x | y | w | h。</summary>
    public static string ButtonRect()
    {
        if (_button == IntPtr.Zero) return "";
        RECT rect;
        GetWindowRect(_button, out rect);
        return "" + rect.left + "|" + rect.top + "|" + (rect.right - rect.left) + "|" + (rect.bottom - rect.top);
    }

    /// <summary>窗口的"看得见的那圈"（DWM 扩展边框，与截图吸附用的是同一个口径）。</summary>
    public static string FrameRect()
    {
        if (_hwnd == IntPtr.Zero) return "";
        RECT dwm;
        int hr = DwmGetWindowAttribute(_hwnd, 9 /* DWMWA_EXTENDED_FRAME_BOUNDS */, out dwm, Marshal.SizeOf(typeof(RECT)));
        if (hr != 0) { GetWindowRect(_hwnd, out dwm); }
        return "" + dwm.left + "|" + dwm.top + "|" + (dwm.right - dwm.left) + "|" + (dwm.bottom - dwm.top);
    }

    /// <summary>按钮左上角在屏幕上的坐标（物理像素）。</summary>
    public static string ButtonScreenRect()
    {
        if (_button == IntPtr.Zero) return "";
        var origin = new POINT { x = 0, y = 0 };
        ClientToScreen(_hwnd, ref origin);
        RECT client;
        GetClientRect(_hwnd, out client);
        int bx = origin.x + ButtonX;
        int by = origin.y + ButtonY;
        return "" + bx + "|" + by + "|" + ButtonW + "|" + ButtonH + "|" + origin.x + "|" + origin.y + "|" + (client.right - client.left) + "|" + (client.bottom - client.top);
    }

    private static IntPtr HandleMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam)
    {
        if (msg == WM_CLOSE) { DestroyWindow(hWnd); return IntPtr.Zero; }
        if (msg == WM_DESTROY)
        {
            PostMessageW(IntPtr.Zero, 0x0012, IntPtr.Zero, IntPtr.Zero);
            return IntPtr.Zero;
        }
        return DefWindowProcW(hWnd, msg, wParam, lParam);
    }
}