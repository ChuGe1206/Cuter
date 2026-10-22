// 脚本化复现「框选 / 滚动」。类名刻意取得独特，避免与 PowerShell 里已加载的类型重名。
//
// 重要：默认情况下 PowerShell 进程是 DPI 不感知的，SetCursorPos / GetWindowRect 拿到的
// 都是「DPI 虚拟化」坐标。先调用 SetPhysicalDpiAwareness() 把当前线程切成 Per-Monitor-V2，
// 之后这些 API 用的就都是真实物理像素，脚本里不必再手动 ×1.25。
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class CuterInputProbe {
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int X, int Y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, uint dwData, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);

    private const uint LEFTDOWN = 0x0002;
    private const uint LEFTUP = 0x0004;
    private const uint RIGHTDOWN = 0x0008;
    private const uint RIGHTUP = 0x0010;
    private const uint MIDDLEDOWN = 0x0020;
    private const uint MIDDLEUP = 0x0040;
    private const uint WHEEL = 0x0800;
    private const uint MOUSEEVENTF_HWHEEL = 0x01000;
    private const uint INPUT_MOUSE = 0;

    [StructLayout(LayoutKind.Sequential)]
    public struct MOUSEINPUT {
        public int dx;
        public int dy;
        public int mouseData;
        public uint dwFlags;
        public uint time;
        public UIntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Explicit)]
    public struct INPUTUNION {
        [FieldOffset(0)] public MOUSEINPUT mi;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT {
        public uint type;
        public INPUTUNION u;
    }

    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint count, INPUT[] inputs, int size);
    private const uint KEYUP = 0x0002;
    private const int WHEEL_DELTA = 120;

    /// DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
    private static readonly IntPtr PER_MONITOR_V2 = new IntPtr(-4);

    /// 让当前线程使用物理像素坐标。返回转换前的上下文（-1 表示调用失败）。
    public static IntPtr SetPhysicalDpiAwareness() {
        try { return SetThreadDpiAwarenessContext(PER_MONITOR_V2); }
        catch (EntryPointNotFoundException) { return IntPtr.Zero; }
    }

    public static bool MoveTo(int x, int y) {
        bool ok = SetCursorPos(x, y);
        Thread.Sleep(50);
        return ok;
    }

    public static void Press() {
        mouse_event(LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(80);
    }

    public static void Release() {
        mouse_event(LEFTUP, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(150);
    }

    /// 单独按下 / 抬起左键（拖出测试要在按住期间插 Ctrl 键与多次移动）。
    public static void LeftDown() { mouse_event(LEFTDOWN, 0, 0, 0, UIntPtr.Zero); Thread.Sleep(60); }
    public static void LeftUp() { mouse_event(LEFTUP, 0, 0, 0, UIntPtr.Zero); Thread.Sleep(60); }
    /// 横向滚轮：`clicks` 为正 = 向右。
    ///
    /// ⚠️ **必须用 `SendInput`，不能用 `mouse_event`**：后者会忽略
    /// `MOUSEEVENTF_HWHEEL`（那是 SendInput 时代才有的标志），事件发出去什么都没发生——
    /// 症状是"横向滚动截屏的探针里目标窗口纹丝不动"（实测踩过）。
    public static void HWheel(int clicks) {
        var input = new INPUT {
            type = INPUT_MOUSE,
            u = new INPUTUNION {
                mi = new MOUSEINPUT {
                    dx = 0,
                    dy = 0,
                    mouseData = clicks * 120,
                    dwFlags = MOUSEEVENTF_HWHEEL,
                    time = 0,
                    dwExtraInfo = UIntPtr.Zero,
                },
            },
        };
        var buffer = new INPUT[1];
        buffer[0] = input;
        SendInput(1, buffer, Marshal.SizeOf(typeof(INPUT)));
        Thread.Sleep(120);
    }

    public static void MiddleDown() { mouse_event(MIDDLEDOWN, 0, 0, 0, UIntPtr.Zero); Thread.Sleep(60); }
    public static void MiddleUp() { mouse_event(MIDDLEUP, 0, 0, 0, UIntPtr.Zero); Thread.Sleep(60); }

    public static void RightDown() { mouse_event(RIGHTDOWN, 0, 0, 0, UIntPtr.Zero); Thread.Sleep(60); }
    public static void RightUp() { mouse_event(RIGHTUP, 0, 0, 0, UIntPtr.Zero); Thread.Sleep(60); }

    /// 在光标当前位置右键一次（托盘图标用：Win32 托盘只认真正的右键）。
    public static void RightClick() { RightDown(); RightUp(); }

    /// 单独按下 / 抬起一个键（同上：需要让按键与鼠标动作交错时用）。
    public static void KeyDown(byte vk) { keybd_event(vk, 0, 0, UIntPtr.Zero); Thread.Sleep(40); }
    public static void KeyUp(byte vk) { keybd_event(vk, 0, KEYUP, UIntPtr.Zero); Thread.Sleep(40); }

    /// 窗口矩形（物理像素）。
    public static RECT WindowRect(IntPtr hwnd) {
        RECT r;
        GetWindowRect(hwnd, out r);
        return r;
    }

    /// 从 (x0,y0) 拖到 (x1,y1)，中间分若干步移动（真实拖拽会产生连续的 mousemove）。
    public static void Drag(int x0, int y0, int x1, int y1) {
        MoveTo(x0, y0);
        // The overlay reads the pointer from its own input queue: press before the move has
        // been processed and the selection starts at the pointer's previous position. That is
        // how a 700x320 drag became 584x267 in scroll_horizontal_e2e (2026-10-17).
        Thread.Sleep(250);
        Press();
        // ...and the same race runs the other way: a move that lands while the button message is
        // still queued makes the *first step* the anchor, so a 700x320 drag came out 642x294
        // (region x=438 instead of 380). A zero-distance move right after the press re-asserts
        // the anchor before the real steps start.
        Thread.Sleep(200);
        MoveTo(x0, y0);
        Thread.Sleep(120);
        int steps = 12;
        for (int i = 1; i <= steps; i++) {
            MoveTo(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps);
        }
        // Same race at the other end: releasing while the final move is still queued makes
        // the overlay end the selection at the second-to-last step. overlay_select_e2e came
        // out 5 px short that way (380x230 expected, 380x235 measured, 2026-10-18).
        Thread.Sleep(150);
        Release();
    }

    public static void CtrlKey(byte vk) { Chord(0x11, vk); }
    public static void AltKey(byte vk) { Chord(0x12, vk); }

    /// 任意修饰键 + 主键（0x10 Shift / 0x11 Ctrl / 0x12 Alt）。Shift+F3、Ctrl+F3 用它。
    public static void Chord(byte modifier, byte vk) {
        keybd_event(modifier, 0, 0, UIntPtr.Zero);
        Thread.Sleep(50);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        Thread.Sleep(50);
        keybd_event(vk, 0, KEYUP, UIntPtr.Zero);
        Thread.Sleep(30);
        keybd_event(modifier, 0, KEYUP, UIntPtr.Zero);
        Thread.Sleep(400);
    }

    /// 单键按一下（0x0D = 回车，0x1B = Esc，0x20 = 空格）。
    public static void Key(byte vk) {
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        Thread.Sleep(40);
        keybd_event(vk, 0, KEYUP, UIntPtr.Zero);
        Thread.Sleep(400);
    }

    /// 滚轮。`clicks` 为正表示向上滚（内容向下移动），为负表示向下滚。
    public static void Wheel(int clicks) {
        for (int i = 0; i < Math.Abs(clicks); i++) {
            mouse_event(WHEEL, 0, 0, (uint)(Math.Sign(clicks) * WHEEL_DELTA), UIntPtr.Zero);
            Thread.Sleep(60);
        }
        Thread.Sleep(150);
    }

    public static bool Activate(IntPtr hwnd) {
        bool ok = SetForegroundWindow(hwnd);
        Thread.Sleep(250);
        return ok;
    }

    // ---- 窗口查找与摆放（坐标一律是物理像素，前提是已经调过 SetPhysicalDpiAwareness） ----

    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int w, int h, bool repaint);

    public struct RECT { public int Left, Top, Right, Bottom; }

    /// 这个窗口是否**真的挡在桌面上**（可见 + 与虚拟桌面有交集）。
    ///
    /// 遮罩窗口为了复用（省掉每次 ~100 ms 的建窗口开销）会停在桌面之外（-32000, 1×1），
    /// 那种窗口 `IsWindowVisible` 同样返回 true。只按可见性判断的探针会把它当成"遮罩已经出现"，
    /// 于是量到的是"停着的窗口"，不是用户看到的那一帧。
    public static bool IsOnDesktop(IntPtr hwnd) {
        if (hwnd == IntPtr.Zero) return false;
        if (!IsWindowVisible(hwnd)) return false;
        RECT r;
        if (!GetWindowRect(hwnd, out r)) return true;
        int vx = GetSystemMetrics(76);   // SM_XVIRTUALSCREEN
        int vy = GetSystemMetrics(77);   // SM_YVIRTUALSCREEN
        int vw = GetSystemMetrics(78);   // SM_CXVIRTUALSCREEN
        int vh = GetSystemMetrics(79);   // SM_CYVIRTUALSCREEN
        return r.Left < vx + vw && r.Right > vx && r.Top < vy + vh && r.Bottom > vy;
    }

    /// 统计某进程里标题匹配且**可见**的顶层窗口个数。
    ///
    /// 贴图分组就是靠这个验证的：切组之后"看得见的贴图窗口"数量应当变化。
    public static int CountWindows(uint pid, string titleContains) {
        int count = 0;
        EnumWindows((h, l) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner != pid) return true;
            if (!IsWindowVisible(h)) return true;
            var sb = new StringBuilder(512);
            GetWindowTextW(h, sb, sb.Capacity);
            if (sb.ToString().IndexOf(titleContains, StringComparison.Ordinal) >= 0) count++;
            return true;
        }, IntPtr.Zero);
        return count;
    }

    /// 按 PID + 标题子串找第一个可见的顶层窗口；找不到返回 IntPtr.Zero。
    public static IntPtr FindWindow(uint pid, string titleContains) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner != pid) return true;
            if (!IsWindowVisible(h)) return true;
            var sb = new StringBuilder(512);
            GetWindowTextW(h, sb, sb.Capacity);
            if (sb.ToString().IndexOf(titleContains, StringComparison.Ordinal) < 0) return true;
            found = h;
            return false;
        }, IntPtr.Zero);
        return found;
    }

    /// `hwnd|x|y|w|h|visible`；窗口不存在时返回空串。
    public static string Describe(IntPtr hwnd) {
        if (hwnd == IntPtr.Zero) return "";
        RECT r;
        if (!GetWindowRect(hwnd, out r)) return "";
        return string.Format("{0}|{1}|{2}|{3}|{4}|{5}",
            hwnd.ToInt64(), r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top, IsWindowVisible(hwnd));
    }

    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT r);

    public struct POINT { public int X, Y; }

    /// 客户区左上角在屏幕上的物理坐标，以及客户区尺寸。
    ///
    /// 屏幕上「窗口矩形」和「客户区」不一定重合：有边框/阴影时客户区会内缩，
    /// 而 egui 收到的是**客户区**坐标 —— 这正是「拖拽起点与截图像素差 1 行」的根源。
    public static string ClientInfo(IntPtr hwnd) {
        if (hwnd == IntPtr.Zero) return "";
        var origin = new POINT { X = 0, Y = 0 };
        if (!ClientToScreen(hwnd, ref origin)) return "";
        RECT client;
        if (!GetClientRect(hwnd, out client)) return "";
        return string.Format("{0},{1}|{2}x{3}", origin.X, origin.Y,
            client.Right - client.Left, client.Bottom - client.Top);
    }

    /// 调用线程当前报告的「屏幕尺寸」。DPI 不感知的线程会拿到被缩放过的假尺寸。
    public static string ScreenSize() {
        return GetSystemMetrics(0) + "x" + GetSystemMetrics(1);
    }

    public static uint DpiOf(IntPtr hwnd) {
        return hwnd == IntPtr.Zero ? 0 : GetDpiForWindow(hwnd);
    }

    /// 0 = DPI_UNAWARE，1 = SYSTEM_AWARE，2 = PER_MONITOR_AWARE。
    ///
    /// 这个值决定 `GetWindowRect` 返回的是物理像素还是被虚拟化的像素：
    /// 返回 0 时必须把窗口坐标乘以 `DpiOf(hwnd) / 96` 才是真实物理尺寸。
    public static int Awareness() {
        try { return GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()); }
        catch (EntryPointNotFoundException) { return -1; }
    }

    // ------------------------------------------------------------------
    // Resource counters (leak probe)
    //
    // GDI/USER object counts are THE classic leak in a screenshot tool: a DC or bitmap
    // that is not released per capture shows up here long before memory does, and the
    // process dies at the 10 000 handle limit much later. GetGuiResources needs
    // PROCESS_QUERY_INFORMATION, which a normal process has on its own children.
    // ------------------------------------------------------------------

    const uint PROCESS_QUERY_INFORMATION = 0x0400;
    const int GR_GDIOBJECTS = 0;
    const int GR_USEROBJECTS = 1;

    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool CloseHandle(IntPtr h);
    [DllImport("user32.dll", SetLastError = true)] public static extern uint GetGuiResources(IntPtr process, int flags);

    // ------------------------------------------------------------------
    // Who owns the keyboard right now?
    //
    // A running VM window (VirtualBox / VMware) or a remote-desktop client **grabs** the
    // keyboard, and then synthesized Alt+C / Enter never reach anybody else - the global
    // hotkey does not fire and the overlay never sees the key. That looks exactly like an
    // app bug ("overlay did not appear"), so probes check this before blaming the app.
    // ------------------------------------------------------------------

    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();

    /// Put the desktop (the shell window) in front.
    ///
    /// A VM or remote-desktop window only *grabs* the keyboard while it is active, so
    /// focusing the desktop releases the grab and synthesized keys reach the system again.
    /// Returns whether the call reported success; callers treat a failure as non-fatal
    /// (they will report SKIPPED instead of a bogus failure).
    public static bool FocusDesktop() {
        IntPtr shell = GetShellWindow();
        if (shell == IntPtr.Zero) return false;
        return SetForegroundWindow(shell);
    }

    public static string ForegroundProcess() {
        IntPtr h = GetForegroundWindow();
        if (h == IntPtr.Zero) return "";
        uint pid;
        GetWindowThreadProcessId(h, out pid);
        try { return System.Diagnostics.Process.GetProcessById((int)pid).ProcessName; }
        catch { return "pid:" + pid; }
    }

    public static string ForegroundTitle() {
        IntPtr h = GetForegroundWindow();
        if (h == IntPtr.Zero) return "";
        var sb = new StringBuilder(256);
        GetWindowTextW(h, sb, 256);
        return sb.ToString();
    }

    /// GDI objects held by the process.
    public static int GdiObjects(uint pid) { return (int)Resources(pid, GR_GDIOBJECTS); }

    /// USER objects (windows, menus, ...) held by the process.
    public static int UserObjects(uint pid) { return (int)Resources(pid, GR_USEROBJECTS); }

    static uint Resources(uint pid, int flag) {
        IntPtr h = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid);
        if (h == IntPtr.Zero) return 0xFFFFFFFF;
        try { return GetGuiResources(h, flag); }
        finally { CloseHandle(h); }
    }
}
