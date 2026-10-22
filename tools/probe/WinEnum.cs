// 按 PID 枚举顶层窗口，避免用中文标题做字符串匹配（命令行编码会干扰）。
// 标题以 UTF-16 码点的十六进制输出，规避控制台代码页问题。
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class WinEnum
{
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hWnd, StringBuilder s, int n);

    public struct RECT { public int Left, Top, Right, Bottom; }

    public static List<string> Dump(uint targetPid)
    {
        var list = new List<string>();
        EnumWindows((h, l) =>
        {
            uint pid;
            GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;

            RECT r;
            GetWindowRect(h, out r);
            var title = new StringBuilder(512);
            GetWindowTextW(h, title, title.Capacity);
            var cls = new StringBuilder(256);
            GetClassNameW(h, cls, cls.Capacity);

            // 标题转成 \uXXXX 序列，彻底避开控制台编码
            var sb = new StringBuilder();
            foreach (char c in title.ToString()) sb.Append("\\u" + ((int)c).ToString("X4"));

            list.Add(string.Format(
                "hwnd=0x{0:X} 可见={1} 类={2} 位置=({3},{4}) 尺寸={5}x{6} 标题={7}",
                h.ToInt64(), IsWindowVisible(h), cls, r.Left, r.Top,
                r.Right - r.Left, r.Bottom - r.Top, sb));
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
