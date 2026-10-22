// 一个"别的程序"：真正的 OLE 放置目标（IDropTarget），用来接收 Cuter 拖出来的文件。
//
// 为什么不用 DragAcceptFiles + WM_DROPFILES：那条老路更短，但实测收不到我们用
// SHCreateDataObject 造的数据（OLE 报 DRAGDROP_S_DROP 但 effect=0，"放下了没人接"）。
// 真实程序（浏览器 / 聊天软件 / 编辑器）几乎都是自己实现 IDropTarget，所以这样测更真。
//
// DragEnter 里 QueryGetData(CF_HDROP)（问得通就说明数据对象确实带着文件），
// Drop 里真的把 HDROP 读出来并逐行写进记录文件。只用于 tools/probe，不进产物。

using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class DropTarget
{
    private const int WM_CLOSE = 0x0010;
    private const int WM_DESTROY = 0x0002;
    private const short CF_HDROP = 15;
    private const uint DROPEFFECT_COPY = 1;

    [StructLayout(LayoutKind.Sequential)]
    private struct POINTL { public int x; public int y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct FORMATETC
    {
        public short cfFormat;
        public IntPtr ptd;
        public uint dwAspect;
        public int lindex;
        public uint tymed;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct STGMEDIUM
    {
        public uint tymed;
        public IntPtr unionmember;
        public IntPtr pUnkForRelease;
    }

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

    private delegate IntPtr WndProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [ComImport, Guid("00000122-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDropTarget
    {
        void DragEnter([MarshalAs(UnmanagedType.Interface)] object data, uint keys, POINTL pt, ref uint effect);
        void DragOver(uint keys, POINTL pt, ref uint effect);
        void DragLeave();
        void Drop([MarshalAs(UnmanagedType.Interface)] object data, uint keys, POINTL pt, ref uint effect);
    }

    [ComImport, Guid("0000010e-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDataObject
    {
        void GetData(ref FORMATETC format, out STGMEDIUM medium);
        void GetDataHere(ref FORMATETC format, ref STGMEDIUM medium);
        int QueryGetData(ref FORMATETC format);
        void GetCanonicalFormatEtc(ref FORMATETC formatIn, out FORMATETC formatOut);
        void SetData(ref FORMATETC format, ref STGMEDIUM medium, bool release);
        void EnumFormatEtc(uint direction, out IntPtr enumFormatEtc);
        void DAdvise(ref FORMATETC format, uint advf, IntPtr adviseSink, out uint connection);
        void DUnadvise(uint connection);
        void EnumDAdvise(out IntPtr enumAdvise);
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern ushort RegisterClassW(ref WNDCLASS lpWndClass);
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
    [DllImport("kernel32.dll")] private static extern IntPtr GlobalLock(IntPtr handle);
    [DllImport("kernel32.dll")] private static extern bool GlobalUnlock(IntPtr handle);
    [DllImport("ole32.dll")] private static extern int OleInitialize(IntPtr reserved);
    [DllImport("ole32.dll")] private static extern int RegisterDragDrop(IntPtr hwnd, IDropTarget target);
    [DllImport("ole32.dll")] private static extern int RevokeDragDrop(IntPtr hwnd);
    [DllImport("ole32.dll")] private static extern void ReleaseStgMedium(ref STGMEDIUM medium);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)] private static extern uint DragQueryFileW(IntPtr hDrop, uint index, StringBuilder buffer, uint size);

    [ComImport, Guid("00000103-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IEnumFORMATETC
    {
        int Next(uint celt, [Out, MarshalAs(UnmanagedType.LPArray)] FORMATETC[] rgelt, IntPtr pceltFetched);
        int Skip(uint celt);
        int Reset();
        int Clone(out IEnumFORMATETC copy);
    }

    /// 在数据对象声明的格式里找出 CF_HDROP 的**精确** FORMATETC。
    /// 真实程序（资源管理器 / 聊天软件）就是这么做的：自己拼一个 FORMATETC
    /// 很容易在 tymed/dwAspect 上对不上，`GetData` 会回 DV_E_FORMATETC。
    private static bool FindHDrop(IDataObject data, out FORMATETC format)
    {
        format = new FORMATETC();
        try
        {
            // PowerShell 5.1 的 Add-Type 是旧编译器：out 变量不能内联声明
            IntPtr raw;
            data.EnumFormatEtc(1 /* DATADIR_GET */, out raw);
            var enumerator = (IEnumFORMATETC)Marshal.GetObjectForIUnknown(raw);
            var buffer = new FORMATETC[8];
            while (enumerator.Next((uint)buffer.Length, buffer, IntPtr.Zero) == 0)
            {
                foreach (var candidate in buffer)
                {
                    if (candidate.cfFormat == CF_HDROP)
                    {
                        format = candidate;
                        return true;
                    }
                }
            }
        }
        catch { }
        return false;
    }

    private sealed class Target : IDropTarget
    {
        public void DragEnter(object data, uint keys, POINTL pt, ref uint effect)
        {
            // QueryGetData 是最省事也最可靠的问法（实测它对我们的数据对象返回 S_OK）。
            // EnumFormatETC 在 PowerShell 5.1 的旧编译器 + COM 互操作下不可靠，只用来说明。
            string verdict;
            var probe = new FORMATETC { cfFormat = CF_HDROP, dwAspect = 1, lindex = -1, tymed = 1 };
            try
            {
                int hr = ((IDataObject)data).QueryGetData(ref probe);
                verdict = hr == 0 ? "yes" : "no(" + hr + ")";
            }
            catch (Exception err) { verdict = "error: " + err.Message; }
            File.AppendAllText(_record, "DRAGENTER cfHDROP=" + verdict + Environment.NewLine);
            effect = DROPEFFECT_COPY;
        }

        public void DragOver(uint keys, POINTL pt, ref uint effect) { effect = DROPEFFECT_COPY; }

        public void DragLeave() { }

        public void Drop(object data, uint keys, POINTL pt, ref uint effect)
        {
            effect = DROPEFFECT_COPY;
            var medium = new STGMEDIUM();
            try
            {
                FORMATETC fmt;
                if (!FindHDrop((IDataObject)data, out fmt))
                {
                    File.AppendAllText(_record, "DROP: no CF_HDROP" + Environment.NewLine);
                    return;
                }
                ((IDataObject)data).GetData(ref fmt, out medium);
                IntPtr hdrop = medium.unionmember;
                IntPtr locked = GlobalLock(hdrop);
                try
                {
                    uint count = DragQueryFileW(hdrop, 0xFFFFFFFF, null, 0);
                    for (uint i = 0; i < count; i++)
                    {
                        uint length = DragQueryFileW(hdrop, i, null, 0);
                        var buffer = new StringBuilder((int)length + 1);
                        DragQueryFileW(hdrop, i, buffer, (uint)buffer.Capacity);
                        File.AppendAllText(_record, buffer.ToString() + Environment.NewLine);
                    }
                    if (count == 0) { File.AppendAllText(_record, "DROP but no files" + Environment.NewLine); }
                }
                finally { GlobalUnlock(hdrop); }
            }
            catch (Exception err)
            {
                File.AppendAllText(_record, "DROP failed: " + err.Message + Environment.NewLine);
            }
            finally
            {
                if (medium.unionmember != IntPtr.Zero) { ReleaseStgMedium(ref medium); }
            }
        }
    }

    private static IntPtr _hwnd = IntPtr.Zero;
    private static string _record = "";
    private static WndProc _proc;
    private static Target _target;

    public static IntPtr Start(string title, int x, int y, int w, int h, string recordPath)
    {
        _record = recordPath;
        var ready = new ManualResetEventSlim(false);
        var thread = new Thread(() =>
        {
            _proc = HandleMessage;
            OleInitialize(IntPtr.Zero);
            var cls = new WNDCLASS
            {
                lpfnWndProc = _proc,
                hInstance = GetModuleHandleW(null),
                lpszClassName = "CuterProbeDropTarget",
            };
            RegisterClassW(ref cls);
            _hwnd = CreateWindowExW(0, "CuterProbeDropTarget", title,
                0x00CF0000, x, y, w, h, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            _target = new Target();
            int hr = RegisterDragDrop(_hwnd, _target);
            File.AppendAllText(_record, "RegisterDragDrop hr=" + hr + Environment.NewLine);
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
            RevokeDragDrop(_hwnd);
            PostMessageW(_hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
            _hwnd = IntPtr.Zero;
        }
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