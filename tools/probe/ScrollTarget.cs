// 一个"可以横向滚动"的窗口：给横向滚动截屏的探针当靶子。
//
// 设计（2026-10-17 重写，两次真机事故之后）：
//   1. **不再用原生 EDIT**：多行 EDIT 即使带 `ES_AUTOHSCROLL` 也会按宽度折行，根本没有横向
//      范围——`EM_LINESCROLL` 和横向滚轮都不可能让它动，探针于是报"横向滚动没生效"。
//   2. **不再自己画**：自绘窗口（`BeginPaint` + `TextOutW`）在 PowerShell 的 C# 封送器下
//      会让进程 0xC0000417 无声消失。现在只用一个 40000 px 宽的 STATIC 装整行长字符，
//      "滚动"就是 `SetWindowPos` 把它往左挪——绘制全部交给系统静态控件（`OcrTarget.cs`
//      已经证明这条路稳）。
//   3. **建窗线程切成 Per-Monitor-V2**：否则 125% 缩放下 `(320,260)+900×420` 会变成
//      `(400,325)+1125×525`，探针按 320/260 算的选区整个偏掉。
// 偏移量 `_scrollX` 是探针能直接读到的真值（`ScrollPos`），不依赖"控件大概滚了"。
// 只用于 tools/probe，不进产物。

using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class ScrollTarget
{
    private const int WM_CLOSE = 0x0010;
    private const int WM_DESTROY = 0x0002;
    private const int WM_MOUSEHWHEEL = 0x020E;
    private const int WM_MOUSEWHEEL = 0x020A;
    private const int WM_SETFONT = 0x0030;
    private const int WS_OVERLAPPEDWINDOW = 0x00CF0000;
    private const int WS_CHILD = 0x40000000;
    private const int WS_VISIBLE = 0x10000000;
    // SS_LEFTNOWORDWRAP = 0x000C：普通 SS_LEFT 的静态控件会在**控件宽度**处折行，
    // 于是"内容"被折成很多行，横向滚动就没意义了。
    private const int SS_LEFTNOWORDWRAP = 0x0000000C;
    private const int SS_NOPREFIX = 0x00000080;
    private const int CONTENT_LENGTH = 6000;
    private const int TEXT_X = 16;
    private const int TEXT_Y = 24;
    private const int TEXT_W = 40000;
    private const int TEXT_H = 60;
    private const uint SWP_NOZORDER = 0x0004;
    private const uint SWP_NOACTIVATE = 0x0010;
    private const uint SWP_SHOWWINDOW = 0x0040;

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
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr SendMessageW(IntPtr hWnd, int msg, IntPtr wParam, IntPtr lParam);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr CreateFontW(int height, int width, int escapement, int orientation, int weight, uint italic, uint underline, uint strikeOut, uint charSet, uint outPrecision, uint clipPrecision, uint quality, uint pitchAndFamily, string face);
    [DllImport("user32.dll")] private static extern bool SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("kernel32.dll")] private static extern IntPtr GetModuleHandleW(string name);

    private static IntPtr _hwnd = IntPtr.Zero;
    private static IntPtr _static = IntPtr.Zero;
    private static IntPtr _font = IntPtr.Zero;
    private static WndProc _proc;
    private static int _scrollX = 0;
    private static Thread _auto;
    private static volatile bool _autoStop = true;
    private static volatile string _error = "";

    /// <summary>内容：伪随机字母（不重复、不周期），所以"横着滚"必然带来新内容。</summary>
    private static string BuildContent()
    {
        var builder = new StringBuilder(CONTENT_LENGTH);
        uint state = 0x12345678u;   // 旧编译器不认数字分隔符
        for (int i = 0; i < CONTENT_LENGTH; i++)
        {
            state = state * 1664525 + 1013904223;
            builder.Append((char)('a' + (int)(state >> 27)));
        }
        return builder.ToString();
    }

    public static IntPtr Start(string title, int x, int y, int w, int h)
    {
        var ready = new ManualResetEventSlim(false);
        _error = "";
        var thread = new Thread(() =>
        {
            // 新线程默认 DPI 不感知：窗口会被系统按缩放虚拟化摆错位置、尺寸也变大。
            SetThreadDpiAwarenessContext(new IntPtr(-4));
            try
            {
                _proc = HandleMessage;
                _font = CreateFontW(-24, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 48, "Consolas");
                var cls = new WNDCLASS
                {
                    lpfnWndProc = _proc,
                    hInstance = GetModuleHandleW(null),
                    hbrBackground = new IntPtr(5),   // COLOR_WINDOW
                    lpszClassName = "CuterProbeScrollTarget",
                };
                RegisterClassW(ref cls);
                _hwnd = CreateWindowExW(0, "CuterProbeScrollTarget", title, WS_OVERLAPPEDWINDOW,
                    x, y, w, h, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (_hwnd == IntPtr.Zero) { _error = "CreateWindowExW failed"; ready.Set(); return; }
                _static = CreateWindowExW(0, "STATIC", BuildContent(),
                    WS_CHILD | WS_VISIBLE | SS_LEFTNOWORDWRAP | SS_NOPREFIX,
                    TEXT_X, TEXT_Y, TEXT_W, TEXT_H,
                    _hwnd, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (_static == IntPtr.Zero) { _error = "creating the STATIC failed"; ready.Set(); return; }
                SendMessageW(_static, WM_SETFONT, _font, new IntPtr(1));
                ShowWindow(_hwnd, 5);
                SetWindowPos(_hwnd, new IntPtr(-1), x, y, w, h, SWP_SHOWWINDOW);
                ready.Set();
                MSG msg;
                while (GetMessageW(out msg, IntPtr.Zero, 0, 0))
                {
                    TranslateMessage(ref msg);
                    DispatchMessageW(ref msg);
                }
            }
            catch (Exception err)
            {
                // 线程里的异常会让整个 PowerShell 进程无声消失（这里踩过），必须自己兜住。
                _error = err.GetType().Name + ": " + err.Message;
                ready.Set();
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
        _autoStop = true;
        if (_hwnd != IntPtr.Zero)
        {
            PostMessageW(_hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
            _hwnd = IntPtr.Zero;
        }
    }

    /// <summary>建窗线程里的异常（探针把它打进日志，避免"脚本无声消失"）。</summary>
    public static string Error() { return _error; }

    /// <summary>已横向滚过的像素。</summary>
    public static int ScrollX() { return _scrollX; }

    /// <summary>当前横向偏移，格式 `x|0`。移动 STATIC 的偏移就是真值。</summary>
    public static string ScrollPos() { return _scrollX.ToString() + "|0"; }

    /// <summary>让靶子自己按固定节奏横向滚（合成滚轮投不进来时的兜底）。</summary>
    public static void StartAutoScroll(int intervalMs, int stepPx)
    {
        _autoStop = true;
        _auto = new Thread(() =>
        {
            _autoStop = false;
            while (!_autoStop)
            {
                Nudge(stepPx);
                Thread.Sleep(intervalMs);
            }
        });
        _auto.IsBackground = true;
        _auto.Start();
    }

    public static void StopAutoScroll() { _autoStop = true; }

    /// <summary>横向挪一段像素：把 40000 px 宽的 STATIC 往左挪，屏幕上就是真的内容在滚。</summary>
    public static void Nudge(int pixels)
    {
        if (pixels == 0) { return; }
        _scrollX += pixels;
        if (_static != IntPtr.Zero)
        {
            SetWindowPos(_static, IntPtr.Zero, TEXT_X - _scrollX, TEXT_Y, TEXT_W, TEXT_H,
                SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    private static IntPtr HandleMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam)
    {
        if (msg == WM_MOUSEHWHEEL)
        {
            int delta = (short)((long)wParam >> 16);
            // 鼠标落在 STATIC 上时滚轮先到子窗口，它不处理会往上冒到这里；
            // 落在外框上时直接到这里。两条路都认。
            Nudge(delta / 120 * 120);
            return IntPtr.Zero;
        }
        if (msg == WM_MOUSEWHEEL)
        {
            // 普通滚轮也横着滚：探针怎么发都能动（早先版本靠这条盖住了"横向滚轮不动"）。
            int delta = (short)((long)wParam >> 16);
            Nudge(-delta / 120 * 120);
            return IntPtr.Zero;
        }
        if (msg == WM_CLOSE) { DestroyWindow(hWnd); return IntPtr.Zero; }
        if (msg == WM_DESTROY)
        {
            PostMessageW(IntPtr.Zero, 0x0012, IntPtr.Zero, IntPtr.Zero);
            return IntPtr.Zero;
        }
        return DefWindowProcW(hWnd, msg, wParam, lParam);
    }
}
