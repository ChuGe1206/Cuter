// 一个"大字号纯 ASCII 文本"的窗口：给 OCR 面板的框/行联动探针当靶子。
//
// 为什么不用记事本：默认字号 + 中英混排时，系统 OCR 有时只认出两行、有时把整段认成乱码，
// 框的行高和行数都不稳定，探针的悬停坐标就只能靠猜（见 docs/HANDOFF.md 69.6）。
// 这里用 70px 的 Consolas 画三行纯 ASCII，行高、行数、每行矩形都是确定的，
// LineRect 直接把屏幕矩形交给探针，探针不用再猜。
// 只用于 tools/probe，不进产物。

using System;
using System.Runtime.InteropServices;
using System.Threading;

public static class OcrTarget
{
    private const int WM_CLOSE = 0x0010;
    private const int WM_DESTROY = 0x0002;
    private const int WM_SETFONT = 0x0030;
    private const uint WS_OVERLAPPEDWINDOW = 0x00CF0000;
    private const uint WS_CHILD = 0x40000000;
    private const uint WS_VISIBLE = 0x10000000;

    private const uint SS_LEFT = 0x00000000;

    public const string LineOne = "ALPHA LINE ONE";
    public const string LineTwo = "BRAVO LINE TWO";
    public const string LineThree = "CHARLIE LINE THREE";

    // 客户区坐标：每行一个 STATIC，位置固定，行高留足（70px 字 + 26px 行距）。
    private const int PadX = 32;
    private const int LineH = 96;
    private const int LineOneY = 24;
    private const int LineTwoY = 132;
    private const int LineThreeY = 240;

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
    [DllImport("user32.dll")] private static extern IntPtr SendMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("kernel32.dll")] private static extern IntPtr GetModuleHandleW(string name);
    // 建窗的线程也要切成 Per-Monitor-V2，否则 CreateWindowEx 的坐标会被 DPI 虚拟化，
    // LineRect 报出来的就不是物理像素（tools/probe/README.md 的坑 1）。
    [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] private static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr CreateFontW(int height, int width, int escapement, int orientation, int weight, uint italic, uint underline, uint strikeOut, uint charSet, uint outPrecision, uint clipPrecision, uint quality, uint pitchAndFamily, string face);

    private const int FW_NORMAL = 400;
    private const uint DEFAULT_CHARSET = 1;
    private const uint OUT_TT_PRECIS = 0;
    private const uint CLIP_DEFAULT_PRECIS = 0;
    private const uint CLEARTYPE_QUALITY = 5;
    private const uint FF_MODERN = 48;

    private static IntPtr _hwnd = IntPtr.Zero;
    private static IntPtr _font = IntPtr.Zero;
    private static readonly IntPtr[] _lines = new IntPtr[3];
    private static string _error = "";
    private static WndProc _proc;

    /// <summary>建窗失败时的原因（空串表示没出错）。</summary>
    public static string Error() { return _error; }

    /// <summary>建一个 820×420 的窗口，三行文本分别落在固定的客户区位置。</summary>
    public static IntPtr Start(int x, int y)
    {
        var ready = new ManualResetEventSlim(false);
        var thread = new Thread(() =>
        {
            try
            {
                try { SetThreadDpiAwarenessContext(new IntPtr(-4)); } catch (EntryPointNotFoundException) { }
                _proc = HandleMessage;
                var cls = new WNDCLASS
                {
                    lpfnWndProc = _proc,
                    hInstance = GetModuleHandleW(null),
                    lpszClassName = "CuterProbeOcrTarget",
                    hbrBackground = new IntPtr(5), // COLOR_WINDOW: 纯白底，OCR 最不容易认错
                };
                RegisterClassW(ref cls);
                _hwnd = CreateWindowExW(0, "CuterProbeOcrTarget", "ocr-target", WS_OVERLAPPEDWINDOW,
                    x, y, 820, 420, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (_hwnd == IntPtr.Zero)
                {
                    _error = "CreateWindowEx failed: " + Marshal.GetLastWin32Error();
                    ready.Set();
                    return;
                }
                _font = CreateFontW(-70, 0, 0, 0, FW_NORMAL, 0, 0, 0, DEFAULT_CHARSET,
                    OUT_TT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, FF_MODERN, "Consolas");
                _lines[0] = CreateLine(LineOne, LineOneY);
                _lines[1] = CreateLine(LineTwo, LineTwoY);
                _lines[2] = CreateLine(LineThree, LineThreeY);
                foreach (var line in _lines) { SendMessageW(line, WM_SETFONT, _font, new IntPtr(1)); }
                ShowWindow(_hwnd, 5);
                ready.Set();
                MSG msg;
                while (GetMessageW(out msg, IntPtr.Zero, 0, 0))
                {
                    TranslateMessage(ref msg);
                    DispatchMessageW(ref msg);
                }
            }
            catch (Exception ex)
            {
                _error = ex.GetType().Name + ": " + ex.Message;
                ready.Set();
            }
        });
        thread.IsBackground = true;
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        ready.Wait(5000);
        if (_hwnd == IntPtr.Zero && _error.Length == 0) { _error = "the target window did not appear within 5s"; }
        return _hwnd;
    }

    public static void Stop()
    {
        if (_hwnd != IntPtr.Zero)
        {
            // 先藏起来再关：WM_CLOSE 是排队的，探针紧接着就要截图，不能赌它已经处理完。
            ShowWindow(_hwnd, 0);
            PostMessageW(_hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
            _hwnd = IntPtr.Zero;
        }
    }

    /// <summary>第 index 行在屏幕上的矩形（物理像素）| x | y | w | h。</summary>
    public static string LineRect(int index)
    {
        if (_hwnd == IntPtr.Zero || index < 0 || index > 2) return "";
        var origin = new POINT { x = PadX, y = 0 };
        ClientToScreen(_hwnd, ref origin);
        int y = index == 0 ? LineOneY : (index == 1 ? LineTwoY : LineThreeY);
        return "" + origin.x + "|" + (origin.y + y) + "|" + LineW + "|" + LineH;
    }

    private const int LineW = 620;

    private static IntPtr CreateLine(string text, int y)
    {
        return CreateWindowExW(0, "STATIC", text, WS_CHILD | WS_VISIBLE | SS_LEFT,
            PadX, y, LineW, LineH, _hwnd, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
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
