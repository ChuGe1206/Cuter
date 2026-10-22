// 列出顶层窗口（含前台窗口）与矩形，用来判断当前屏幕上是什么
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class WinList {
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hWnd, StringBuilder s, int n);

    public struct RECT { public int Left, Top, Right, Bottom; }

    public static List<string> Visible() {
        var list = new List<string>();
        IntPtr fg = GetForegroundWindow();
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h) || IsIconic(h)) return true;
            RECT r; GetWindowRect(h, out r);
            if (r.Right - r.Left <= 1 || r.Bottom - r.Top <= 1) return true;
            var t = new StringBuilder(512); GetWindowTextW(h, t, t.Capacity);
            var c = new StringBuilder(256); GetClassNameW(h, c, c.Capacity);
            uint pid; GetWindowThreadProcessId(h, out pid);
            var esc = new StringBuilder();
            foreach (char ch in t.ToString()) esc.Append("\\u" + ((int)ch).ToString("X4"));
            list.Add(string.Format("{0} pid={1} 类={2} 位置=({3},{4}) 尺寸={5}x{6} 标题={7}",
                h == fg ? "[前台]" : "      ", pid, c, r.Left, r.Top,
                r.Right - r.Left, r.Bottom - r.Top, esc));
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
