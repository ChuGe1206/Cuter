// 关键：GetWindowRect 返回的是「调用进程 DPI 上下文」里的坐标。
// PowerShell 是 DPI 不感知的，所以在 125% 缩放下会把 1920x1080 报告成 1536x864。
// 这里用 GetDpiForWindow 把缩放比还原出来，得到真实的物理尺寸。
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class WinDpi
{
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int n);

    public struct RECT { public int Left, Top, Right, Bottom; }

    public static List<string> Dump(uint targetPid, string mustContainEscaped)
    {
        var list = new List<string>();
        EnumWindows((h, l) =>
        {
            uint pid;
            GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;

            var title = new StringBuilder(512);
            GetWindowTextW(h, title, title.Capacity);
            var sb = new StringBuilder();
            foreach (char c in title.ToString()) sb.Append("\\u" + ((int)c).ToString("X4"));
            string escaped = sb.ToString();
            if (mustContainEscaped != null && !escaped.Contains(mustContainEscaped)) return true;

            RECT r;
            GetWindowRect(h, out r);
            uint dpi = GetDpiForWindow(h);
            double s = dpi / 96.0;

            list.Add(string.Format(
                "hwnd=0x{0:X} 可见={1} dpi={2}({3:P0}) 报告尺寸={4}x{5} 真实物理尺寸={6}x{7} 报告位置=({8},{9}) 真实物理位置=({10},{11})",
                h.ToInt64(), IsWindowVisible(h), dpi, s,
                r.Right - r.Left, r.Bottom - r.Top,
                Math.Round((r.Right - r.Left) * s), Math.Round((r.Bottom - r.Top) * s),
                r.Left, r.Top,
                Math.Round(r.Left * s), Math.Round(r.Top * s)));
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
