//! Windows 平台屏幕捕获后端（GDI `BitBlt`）。
//!
//! 选择纯 GDI 而不是 Desktop Duplication API 的理由：
//!
//! * 一次 `BitBlt` 就能拿到整个虚拟桌面（含负坐标的多屏布局），不需要逐屏、逐帧编排；
//! * 兼容 Windows 8.1+（`GetDpiForMonitor` 静态导入 `shcore.dll`，因此不是 Win7）、
//!   远程桌面与虚拟显卡，不依赖 `IDXGIOutputDuplication`；
//! * `CreateDIBSection` 让系统把像素直接映射到我们手里，捕获路径只有一次内存拷贝。
//!
//! 代价是 GDI 不适合长时间高频连拍（冻结截图 / 滚动截屏的增量帧完全够用），
//! 也拿不到 DirectX 独占全屏的画面——那类场景需要另开 DXGI 后端。
//!
//! 坐标系与 crate 约定一致：**全局物理像素**，原点在虚拟桌面左上角，最左侧显示器
//! 在主屏左边时其 `x` 为负数。所有 GDI / 系统句柄都由 RAII guard 持有，
//! 因此任何提前 `return` 或 `?` 都不会泄漏资源。

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Once;

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, GetLastError, LPARAM, POINT, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GdiFlush, GetDC, GetMonitorInfoW,
    HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFOEXW, ROP_CODE, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, MONITORINFOF_PRIMARY};

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::geom::{Point, Rect};

use super::{DesktopShot, MonitorInfo, ScreenCapture, virtual_bounds};

// ---------------------------------------------------------------------------
// DPI 感知
// ---------------------------------------------------------------------------

/// 进程级 DPI 感知的初始化状态；`Once` 保证真正的 API 调用只发生一次。
static DPI_AWARENESS_INIT: Once = Once::new();

/// 在创建任何窗口之前把进程设置为 Per-Monitor-V2 DPI 感知。
///
/// 必须在 `eframe` 初始化之前调用，否则 Windows 会对窗口做位图拉伸，冻结截图与屏幕
/// 像素不再一一对应（125% / 150% 缩放下最明显）。重复调用安全，且永不失败：
/// DPI 感知已被 manifest 或更早的调用设定时只记录一条 debug 日志。
pub fn ensure_dpi_awareness() {
    DPI_AWARENESS_INIT.call_once(|| {
        // SAFETY: 只传入常量（-4，即 PER_MONITOR_AWARE_V2），不涉及任何指针；
        // 该 API 在进程生命周期内随时可调，重复调用由上面的 Once 挡掉。
        let outcome =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        match outcome {
            Ok(()) => log::debug!("已启用 Per-Monitor-V2 DPI 感知"),
            // 被 manifest 或更早的调用设定过时返回 ERROR_ACCESS_DENIED，属正常情况。
            Err(err) if win32_code(&err) == ERROR_ACCESS_DENIED.0 => {
                log::debug!("DPI 感知已由 manifest 或更早的调用设定，忽略：{err}");
            }
            Err(err) => log::warn!("设置 DPI 感知失败（错误码 {}）：{err}", win32_code(&err)),
        }
    });
}

// ---------------------------------------------------------------------------
// 后端
// ---------------------------------------------------------------------------

/// Windows 的 GDI 捕获后端。
///
/// 结构体刻意不带字段：GDI 句柄一律「用时申请、用完立刻释放」，既不缓存可能失效的
/// `HDC`（显示器热插拔后旧句柄会失效），也让类型天然满足 `Send + Sync`。
///
/// 刻意不实现 `Default`：构造后端必须经过 [`WindowsCapture::new`]，否则进程的 DPI
/// 感知不会被设置，截图与屏幕像素就会错位。
#[derive(Debug)]
pub struct WindowsCapture;

impl WindowsCapture {
    /// 创建后端；顺带确保进程已开启 DPI 感知（幂等）。
    pub fn new() -> Result<Self> {
        ensure_dpi_awareness();
        Ok(Self)
    }
}

impl ScreenCapture for WindowsCapture {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        enumerate_monitors()
    }

    fn capture_desktop(&self) -> Result<DesktopShot> {
        let monitors = self.monitors()?;
        let bounds = virtual_bounds(&monitors);
        let frame = capture_rect_impl(bounds)?;
        Ok(DesktopShot {
            frame,
            bounds,
            monitors,
            cursor: cursor_position(),
        })
    }

    fn capture_region(&self, region: Rect) -> Result<Frame> {
        // 空区域不必惊动 GDI：直接按接口约定报错。
        if region.is_empty() {
            return Err(Error::EmptyRegion(region));
        }
        capture_rect_impl(region)
    }

    fn backend_name(&self) -> &'static str {
        "Windows GDI (BitBlt)"
    }
}

// ---------------------------------------------------------------------------
// 显示器枚举
// ---------------------------------------------------------------------------

/// `EnumDisplayMonitors` 的回调：把每个 `HMONITOR` 追加进 `lparam` 指向的 `Vec`。
///
/// # Safety
///
/// `lparam` 必须是由 [`enumerate_monitors`] 传入的 `*mut Vec<HMONITOR>`，该 `Vec`
/// 在整个枚举期间存活，且枚举期间没有其它线程访问它（GDI 串行调用本回调）。
unsafe extern "system" fn enum_monitor_proc(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    // SAFETY: 见函数文档——lparam 指向一个存活且独占的 Vec<HMONITOR>。
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    // 返回 TRUE 让 GDI 继续枚举下一台显示器。
    TRUE
}

/// 枚举全部显示器，按左上角坐标排序（顺序稳定，便于 UI 与配置持久化）。
fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: 回调签名与 MONITORENUMPROC 一致；lparam 指向本函数栈上的 handles，
    // 它在 unsafe 块结束前一直存活（既未移动也未重新赋值），GDI 不会并发调用回调。
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_proc),
            LPARAM((&raw mut handles) as isize),
        )
    };
    if !ok.as_bool() {
        return Err(Error::Win32 {
            api: "EnumDisplayMonitors",
            code: last_error(),
        });
    }

    let mut monitors = Vec::with_capacity(handles.len());
    for handle in handles {
        // 单台显示器查询失败（枚举过程中被拔掉、驱动抽风）不应让整次捕获失败。
        match query_monitor(handle) {
            Ok(info) => monitors.push(info),
            Err(err) => log::warn!("跳过查询失败的显示器 {:p}：{err}", handle.0),
        }
    }
    if monitors.is_empty() {
        return Err(Error::NoMonitor);
    }
    sort_monitors(&mut monitors);
    Ok(monitors)
}

/// 查询单台显示器的静态信息。
fn query_monitor(handle: HMONITOR) -> Result<MonitorInfo> {
    // MONITORINFOEXW 只在 MONITORINFO 后面多了一个设备名字段，且必须以 MONITORINFOEXW
    // 的大小填充 cbSize（Win32 靠它区分调用方给的是哪个版本）。
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    // SAFETY: info 已按要求填好 cbSize；MONITORINFOEXW 的首字段就是 MONITORINFO，
    // 因此把 &mut info.monitorInfo 当作 *mut MONITORINFO 交给系统是合法的（win32 惯例）。
    let ok = unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) };
    if !ok.as_bool() {
        return Err(Error::Win32 {
            api: "GetMonitorInfoW",
            code: last_error(),
        });
    }

    Ok(MonitorInfo {
        // HMONITOR 在进程生命周期内对同一台显示器稳定，作为跨会话的 id 使用。
        id: handle.0 as u64,
        name: device_name(&info.szDevice),
        bounds: rect_from_win32(&info.monitorInfo.rcMonitor),
        work_area: rect_from_win32(&info.monitorInfo.rcWork),
        scale_factor: scale_factor_for(handle),
        is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

/// 该显示器的有效 DPI 缩放；查询失败时保守回退到 1.0。
///
/// 缩放只影响 UI 展示与逻辑尺寸计算（`MonitorInfo::logical_size`），捕获到的像素
/// 始终是物理像素，所以回退值不会污染坐标。
fn scale_factor_for(handle: HMONITOR) -> f32 {
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    // SAFETY: dpi_x / dpi_y 是有效的可写 u32；handle 来自 EnumDisplayMonitors，仍然有效。
    match unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
        Ok(()) => scale_factor_from_dpi(dpi_x, dpi_y),
        Err(err) => {
            log::debug!("GetDpiForMonitor 失败，按 100% 处理：{err}");
            1.0
        }
    }
}

/// `GetDpiForMonitor` 的横纵 DPI → 缩放系数（96 DPI = 100%）。
///
/// 同一个显示器的横纵 DPI 恒等（Windows 不支持非等比缩放），取较大值只是为了让
/// 「成功但返回 0」的驱动不至于算出 0.0。
fn scale_factor_from_dpi(dpi_x: u32, dpi_y: u32) -> f32 {
    let dpi = dpi_x.max(dpi_y);
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// 按左上角坐标排序。`EnumDisplayMonitors` 的顺序取决于驱动报告适配器的顺序，
/// 可能随驱动更新变化；`id` 作为兜底键，处理镜像显示器这种同原点布局。
fn sort_monitors(monitors: &mut [MonitorInfo]) {
    monitors.sort_by_key(|m| (m.bounds.x, m.bounds.y, m.id));
}

/// `MONITORINFOEXW::szDevice`（UTF-16、以 0 结尾、最多 32 个码元）→ `String`。
fn device_name(sz_device: &[u16]) -> String {
    let len = sz_device
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(sz_device.len());
    String::from_utf16_lossy(&sz_device[..len])
}

/// `RECT` → [`Rect`]。
///
/// 两者对边界的约定完全一致：`RECT::right` / `bottom` 与 [`Rect`] 一样指向最后一个
/// 像素之外（右下角开区间），因此直接 `from_ltrb` 即可，不需要 ±1 修正；
/// `right <= left` 或 `bottom <= top` 会自动退化成空矩形。
fn rect_from_win32(rect: &RECT) -> Rect {
    Rect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom)
}

/// 捕获瞬间的鼠标位置（全局物理坐标）；没有输入桌面（服务会话）时返回 `None`。
fn cursor_position() -> Option<Point> {
    let mut point = POINT::default();
    // SAFETY: point 是有效的可写 POINT。
    match unsafe { GetCursorPos(&mut point) } {
        Ok(()) => Some(Point::new(point.x, point.y)),
        Err(err) => {
            log::debug!("GetCursorPos 失败，本次截图不携带光标位置：{err}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// 像素捕获
// ---------------------------------------------------------------------------

/// 捕获一块全局物理像素区域。
///
/// 性能要点：`CreateDIBSection` 使用**顶向下**（`biHeight` 为负）的 32bpp DIB，
/// 行序与 [`Frame`] 完全一致，系统直接把像素映射到我们手里的指针上，
/// 于是 `BitBlt` 的结果不需要再经 `GetDIBits` 二次拷贝——全程只有最后构造 `Frame`
/// 时那一次 `to_vec()`。
fn capture_rect_impl(region: Rect) -> Result<Frame> {
    if region.is_empty() {
        return Err(Error::EmptyRegion(region));
    }

    let screen_dc = ScreenDc::acquire()?;
    let memory_dc = MemoryDc::create(screen_dc.0)?;
    let dib = DibSection::create(screen_dc.0, region.w, region.h)?;

    // SAFETY: dib 的位图与 memory_dc 兼容（都由同一个屏幕 DC 创建），换入后由
    // SelectedObject 负责换回；返回值（原来的位图）正好是需要保存的那个。
    let previous = unsafe { SelectObject(memory_dc.0, HGDIOBJ(dib.bitmap.0)) };
    let selection = SelectedObject {
        dc: memory_dc.0,
        previous,
    };

    // CAPTUREBLT 是必需的：没有它捕获不到分层窗口（某些工具提示、半透明 UI、
    // 屏幕键盘），截图里会出现「空洞」。ROP_CODE 是 newtype，只能按值位或回来。
    let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
    // SAFETY: memory_dc 已选入尺寸为 region 的 DIB；screen_dc 是覆盖整个虚拟桌面的
    // 屏幕 DC，region 的负坐标（多屏布局）对它是合法的源坐标。
    unsafe {
        BitBlt(
            memory_dc.0,
            0,
            0,
            region.w as i32,
            region.h as i32,
            Some(screen_dc.0),
            region.x,
            region.y,
            rop,
        )
    }
    .map_err(|err| win32_error("BitBlt", &err))?;

    // SAFETY: GdiFlush 无参数、无副作用，只是把 GDI 的批处理队列刷完，
    // 保证 bits 指向的像素已经写完（BitBlt 之后读像素前必须调用）。
    let _ = unsafe { GdiFlush() };

    let len = region.size().rgba_len();
    // SAFETY: bits 由 CreateDIBSection 映射、长度恰为 w*h*4（32bpp、顶向下、紧密排列，
    // 与 Frame 的布局一致），在 dib 被删除前一直有效；这里立即拷出，不保留该指针。
    let data = unsafe { std::slice::from_raw_parts(dib.bits.cast::<u8>(), len) }.to_vec();

    // 显式收尾，顺序不能反：先把 DC 恢复原状，再删除仍然被选中的位图。
    drop(selection);
    drop(dib);

    // from_bgrx 负责 BGRA→RGBA 与 alpha=255。
    Frame::from_bgrx(region.size(), data)
}

/// `GetDC(NULL)` 借出的屏幕 DC，离开作用域时自动归还。
struct ScreenDc(HDC);

impl ScreenDc {
    /// 获取覆盖整个虚拟桌面的屏幕 DC。
    fn acquire() -> Result<Self> {
        // SAFETY: 传 None 表示整个屏幕（虚拟桌面）的 DC；返回空句柄表示失败。
        let hdc = unsafe { GetDC(None) };
        if hdc.is_invalid() {
            return Err(Error::Win32 {
                api: "GetDC",
                code: last_error(),
            });
        }
        Ok(Self(hdc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: 句柄由 GetDC(None) 成功返回，且本结构体是它的唯一所有者，
        // 因此只会归还一次；hwnd 参数必须与申请时一致（同样是 None）。
        let _ = unsafe { ReleaseDC(None, self.0) };
    }
}

/// `CreateCompatibleDC` 创建的内存 DC，离开作用域时自动删除。
struct MemoryDc(HDC);

impl MemoryDc {
    fn create(reference: HDC) -> Result<Self> {
        // SAFETY: reference 是有效的 DC（由 ScreenDc 保证在本次调用期间存活）。
        let hdc = unsafe { CreateCompatibleDC(Some(reference)) };
        if hdc.is_invalid() {
            return Err(Error::Win32 {
                api: "CreateCompatibleDC",
                code: last_error(),
            });
        }
        Ok(Self(hdc))
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: 句柄由 CreateCompatibleDC 创建，且本结构体独占所有权、只会删除一次。
        let _ = unsafe { DeleteDC(self.0) };
    }
}

/// `CreateDIBSection` 得到的位图，离开作用域时 `DeleteObject`。
///
/// 同时持有系统映射出来的像素首地址 `bits`：该地址在删除位图前一直有效，
/// 且不受 `SelectObject` / `BitBlt` 影响（它属于位图，不属于 DC）。
struct DibSection {
    bitmap: HBITMAP,
    bits: *mut c_void,
}

impl DibSection {
    /// 创建一张 `width × height` 的 32bpp 顶向下 DIB（即 `BGRX` 布局）。
    fn create(reference: HDC, width: u32, height: u32) -> Result<Self> {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width as i32;
        // 负高度 = 顶向下：第 0 行就是屏幕最上面那一行，与 Frame 的行序一致，
        // 免去上下翻转。正高度会得到自下而上的 DIB。
        bmi.bmiHeader.biHeight = -(height as i32);
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        // 注意 windows-rs 0.62 的不一致：字段是裸 u32，而 BI_RGB 是 BI_COMPRESSION
        // newtype，所以要取 .0 才能赋值。
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: bmi 已按 32bpp 顶向下 DIB 正确初始化；bits 是有效的可写局部变量；
        // 不传入文件映射对象（None）表示由系统分配内存，offset 必须为 0。
        let bitmap = unsafe {
            CreateDIBSection(Some(reference), &bmi, DIB_RGB_COLORS, &mut bits, None, 0u32)
        }
        .map_err(|err| win32_error("CreateDIBSection", &err))?;

        Ok(Self { bitmap, bits })
    }
}

impl Drop for DibSection {
    fn drop(&mut self) {
        // SAFETY: bitmap 由 CreateDIBSection 创建，本结构体独占所有权；
        // 调用方必须在删除前先把它从 DC 里换出去（见 capture_rect_impl 的收尾顺序）。
        let _ = unsafe { DeleteObject(HGDIOBJ(self.bitmap.0)) };
    }
}

/// 记录 `SelectObject` 换进 DC 之前的旧对象，离开作用域时换回去。
///
/// 顺序很重要：必须在 `DeleteObject(dib)` **之前**还原，否则 DC 会继续引用一张
/// 已被删除的位图（GDI 会画出随机内容，甚至让后续 `BitBlt` 失败）。
struct SelectedObject {
    dc: HDC,
    previous: HGDIOBJ,
}

impl Drop for SelectedObject {
    fn drop(&mut self) {
        // SAFETY: dc 在本结构体存活期间保持有效，previous 正是它原本的 GDI 对象
        // （CreateCompatibleDC 出来的 DC 初始持有 1x1 单色位图，不会是空句柄）。
        let _ = unsafe { SelectObject(self.dc, self.previous) };
    }
}

// ---------------------------------------------------------------------------
// windows-rs 错误码转换
// ---------------------------------------------------------------------------

/// 最近一次 Win32 错误码；用于不返回 `Result` 的老式 API（`GetDC`、`GetMonitorInfoW` …）。
fn last_error() -> u32 {
    // SAFETY: GetLastError 无参数、无副作用，只读当前线程的错误码。
    unsafe { GetLastError() }.0
}

/// 把 windows-rs 的错误转成本 crate 的 [`Error::Win32`]（错误码已还原为原始 Win32 码）。
fn win32_error(api: &'static str, err: &windows::core::Error) -> Error {
    Error::Win32 {
        api,
        code: win32_code(err),
    }
}

/// 从 [`windows::core::Error`] 还原原始 Win32 错误码。
///
/// 生成的绑定会把失败的 `BOOL` 变成 `Error::from_thread()`，其中 `GetLastError()` 的
/// 值被 `HRESULT::from_win32` 塞进了 FACILITY_WIN32（`0x8007_xxxx`）。直接打印
/// HRESULT 与 MSDN 上的错误码对不上，所以这里把高位剥掉。
fn win32_code(err: &windows::core::Error) -> u32 {
    let hr = err.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;

    #[test]
    fn ensure_dpi_awareness_is_idempotent() {
        // 会在 eframe 初始化之前被调用，因此必须永不 panic；重复调用由内部 Once 挡掉。
        ensure_dpi_awareness();
        ensure_dpi_awareness();
        ensure_dpi_awareness();
        // new() 也会调用它一次
        assert!(WindowsCapture::new().is_ok());
    }

    #[test]
    fn monitors_are_consistent() {
        let capture = WindowsCapture::new().expect("Windows 平台必然可以创建 GDI 后端");
        let monitors = capture.monitors().expect("至少应枚举到一台显示器");
        assert!(!monitors.is_empty(), "显示器列表不应为空");

        for m in &monitors {
            assert!(
                m.bounds.w > 0 && m.bounds.h > 0,
                "显示器 {} 的尺寸非法：{:?}",
                m.name,
                m.bounds
            );
            assert!(
                m.scale_factor > 0.0,
                "显示器 {} 的缩放非法：{}",
                m.name,
                m.scale_factor
            );
            assert!(
                m.work_area.w > 0 && m.work_area.h > 0,
                "显示器 {} 的工作区非法：{:?}",
                m.name,
                m.work_area
            );
            assert!(!m.name.is_empty(), "显示器设备名不应为空");
            assert_ne!(m.id, 0, "HMONITOR 句柄不应为空");
        }

        let primary = monitors.iter().filter(|m| m.is_primary).count();
        assert_eq!(primary, 1, "应恰好有一台主显示器，实际 {primary} 台");

        // 排序不变量：按左上角坐标单调不减
        assert!(
            monitors
                .windows(2)
                .all(|w| (w[0].bounds.x, w[0].bounds.y) <= (w[1].bounds.x, w[1].bounds.y)),
            "显示器未按左上角坐标排序：{monitors:?}"
        );

        // 虚拟桌面必须覆盖每一台显示器
        let desktop = virtual_bounds(&monitors);
        for m in &monitors {
            assert!(
                desktop.contains_rect(m.bounds),
                "虚拟桌面 {desktop:?} 未覆盖显示器 {} 的 {:?}",
                m.name,
                m.bounds
            );
        }
    }

    #[test]
    fn rect_from_win32_supports_negative_origin() {
        let rect = RECT {
            left: -1920,
            top: -100,
            right: 0,
            bottom: 980,
        };
        assert_eq!(rect_from_win32(&rect), Rect::new(-1920, -100, 1920, 1080));
    }

    #[test]
    fn rect_from_win32_degenerates_to_empty() {
        // right < left / bottom < top 都是非法区间，from_ltrb 会退化成空矩形
        assert!(
            rect_from_win32(&RECT {
                left: 10,
                top: 10,
                right: 5,
                bottom: 5,
            })
            .is_empty()
        );
        assert!(rect_from_win32(&RECT::default()).is_empty());
    }

    #[test]
    fn rect_from_win32_keeps_half_open_right_bottom() {
        let rect = rect_from_win32(&RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        });
        assert_eq!((rect.right(), rect.bottom()), (1920, 1080));
        assert!(rect.contains(Point::new(1919, 1079)));
        assert!(!rect.contains(Point::new(1920, 1080)));
    }

    #[test]
    fn device_name_stops_at_nul_and_tolerates_full_buffer() {
        let mut buf = [0u16; 32];
        for (slot, ch) in buf.iter_mut().zip("\\\\.\\DISPLAY1".encode_utf16()) {
            *slot = ch;
        }
        assert_eq!(device_name(&buf), "\\\\.\\DISPLAY1");

        // 没有结尾 0（驱动给了满缓冲）时也要安全返回全部内容
        assert_eq!(device_name(&[b'A' as u16; 32]), "A".repeat(32));
        assert_eq!(device_name(&[]), "");
    }

    #[test]
    fn scale_factor_is_relative_to_96_dpi() {
        assert!((scale_factor_from_dpi(96, 96) - 1.0).abs() < f32::EPSILON);
        assert!((scale_factor_from_dpi(144, 144) - 1.5).abs() < f32::EPSILON);
        assert!((scale_factor_from_dpi(240, 240) - 2.5).abs() < f32::EPSILON);
        // 驱动返回 0 时不能算出 0.0（否则 logical_size 会除零）
        assert!((scale_factor_from_dpi(0, 0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sort_monitors_orders_left_to_right() {
        let fake = |x: i32, y: i32, id: u64| MonitorInfo {
            id,
            name: String::new(),
            bounds: Rect::new(x, y, 10, 10),
            work_area: Rect::new(x, y, 10, 10),
            scale_factor: 1.0,
            is_primary: false,
        };
        let mut list = vec![fake(1920, 0, 3), fake(0, 0, 1), fake(-1920, 120, 2)];
        sort_monitors(&mut list);
        assert_eq!(
            list.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![2, 1, 3],
            "应严格按 (x, y) 从左上到右下排序"
        );
    }

    #[test]
    fn win32_code_unwraps_facility_win32_hresult() {
        let access_denied = windows::core::Error::from_hresult(windows::core::HRESULT(
            (0x8007_0000u32 | ERROR_ACCESS_DENIED.0) as i32,
        ));
        assert_eq!(win32_code(&access_denied), ERROR_ACCESS_DENIED.0);
        // 错误信息里应是原始 Win32 码（5），而不是 0x80070005
        let message = win32_error("BitBlt", &access_denied).to_string();
        assert!(
            message.contains("错误码 5"),
            "错误信息应带原始 Win32 错误码，实际：{message}"
        );
    }

    #[test]
    fn empty_region_is_rejected_without_touching_gdi() {
        let capture = WindowsCapture::new().unwrap();
        let err = capture.capture_region(Rect::ZERO).unwrap_err();
        assert!(matches!(err, Error::EmptyRegion(r) if r == Rect::ZERO));
    }

    /// `COLORREF`（`0x00BBGGRR`，注意与 RGB 相反）→ `(r, g, b)`。
    fn colorref_to_rgb(color: u32) -> (u8, u8, u8) {
        (
            (color & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            ((color >> 16) & 0xFF) as u8,
        )
    }

    #[test]
    fn colorref_decoding_is_not_channel_swapped() {
        // COLORREF 的低字节是红：0x00112233 表示 r=0x33, g=0x22, b=0x11
        assert_eq!(colorref_to_rgb(0x0011_2233), (0x33, 0x22, 0x11));
        assert_eq!(colorref_to_rgb(0x00FF_0000), (0, 0, 255));
    }

    /// 用 `GetPixel` 读同一坐标作为基准，逐点校验截图内容。
    ///
    /// 这是唯一能同时证明「`BitBlt` 真的抓到了画面」与「`BGRA`→`RGBA` 通道顺序、
    /// 全局→局部坐标映射都没错位」的测试：通道反了或坐标偏了都会让一致率变成 0。
    #[test]
    #[ignore = "需要交互式桌面（用 GetPixel 逐点校验截图内容）"]
    fn captured_pixels_match_gdi_ground_truth() {
        use windows::Win32::Graphics::Gdi::{CLR_INVALID, GetPixel};

        let capture = WindowsCapture::new().unwrap();
        let shot = capture.capture_desktop().unwrap();
        let screen_dc = ScreenDc::acquire().unwrap();

        // 采样点分散在主屏四角、中心与任务栏附近
        let points = [
            Point::new(5, 5),
            Point::new(100, 100),
            Point::new(960, 540),
            Point::new(1500, 900),
            Point::new(1700, 200),
            Point::new(50, 900),
            Point::new(300, 400),
        ];
        let mut compared = 0;
        let mut matched = 0;
        for global in points {
            if !shot.bounds.contains(global) {
                continue;
            }
            // SAFETY: screen_dc 有效，global 是虚拟桌面内的物理坐标。
            let color = unsafe { GetPixel(screen_dc.0, global.x, global.y) };
            if color.0 == CLR_INVALID {
                continue;
            }
            let (r, g, b) = colorref_to_rgb(color.0);
            let local = shot.to_local(global);
            let px = shot.frame.pixel(local.x, local.y);
            compared += 1;
            if px[0] == r && px[1] == g && px[2] == b {
                matched += 1;
            }
        }

        assert!(
            compared >= 4,
            "至少要有 4 个可比较的采样点，实际 {compared}"
        );
        // 时钟、悬停高亮、翻页动画会让个别点在两次读取之间变化，所以不要求 100%；
        // 但通道错序或坐标错位会让一致率直接掉到 0。
        assert!(
            matched * 2 >= compared,
            "只有 {matched}/{compared} 个采样点与 GDI 基准一致，通道顺序或坐标映射可能有误"
        );
        println!("GetPixel 逐点校验：{matched}/{compared} 个采样点完全一致");
    }

    /// 拆开量一遍捕获链路的每一步，回答「53~105 ms 到底花在哪」。
    ///
    /// 只打印不打断言（它是一把尺子，不是门槛），用
    /// `cargo test -p cuter-core bench_capture_breakdown -- --ignored --nocapture` 跑。
    #[test]
    #[ignore = "需要交互式桌面，且要独占屏幕；只在手动调优时跑"]
    fn bench_capture_breakdown() {
        use std::time::Instant;

        let capture = WindowsCapture::new().unwrap();
        let shot = capture.capture_desktop().unwrap();
        let region = shot.bounds;
        let screen_dc = ScreenDc::acquire().unwrap();
        let memory_dc = MemoryDc::create(screen_dc.0).unwrap();
        let dib = DibSection::create(screen_dc.0, region.w, region.h).unwrap();
        let len = region.size().rgba_len();

        let rounds = 20;
        fn time(label: &str, rounds: u32, mut body: impl FnMut()) {
            body(); // 预热
            let start = Instant::now();
            for _ in 0..rounds {
                body();
            }
            let per = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
            println!("{label:<28} {per:8.2} ms");
        }

        time("BitBlt + CAPTUREBLT", rounds, || {
            let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
            unsafe {
                BitBlt(
                    memory_dc.0,
                    0,
                    0,
                    region.w as i32,
                    region.h as i32,
                    Some(screen_dc.0),
                    region.x,
                    region.y,
                    rop,
                )
            }
            .unwrap();
        });
        time("BitBlt (no CAPTUREBLT)", rounds, || {
            unsafe {
                BitBlt(
                    memory_dc.0,
                    0,
                    0,
                    region.w as i32,
                    region.h as i32,
                    Some(screen_dc.0),
                    region.x,
                    region.y,
                    SRCCOPY,
                )
            }
            .unwrap();
        });
        time("GdiFlush", rounds, || unsafe {
            let _ = GdiFlush();
        });
        time("to_vec (8 MB)", rounds, || {
            let data = unsafe { std::slice::from_raw_parts(dib.bits.cast::<u8>(), len) }.to_vec();
            std::hint::black_box(&data);
        });
        let raw = unsafe { std::slice::from_raw_parts(dib.bits.cast::<u8>(), len) }.to_vec();
        time("from_bgrx (swizzle)", rounds, || {
            let copy = raw.clone();
            std::hint::black_box(Frame::from_bgrx(region.size(), copy).unwrap());
        });
        time("capture_rect_impl (total)", rounds, || {
            std::hint::black_box(capture_rect_impl(region).unwrap());
        });
        time("capture_desktop (whole)", rounds, || {
            std::hint::black_box(capture.capture_desktop().unwrap());
        });
        let detector = crate::capture::window_detector().unwrap();
        time("window_detector", rounds, || {
            std::hint::black_box(detector.visible_windows().unwrap());
        });
        time("monitors()", rounds, || {
            std::hint::black_box(capture.monitors().unwrap());
        });

        println!(
            "屏幕 {}×{}，每次搬运 {:.1} MB",
            region.w,
            region.h,
            len as f64 / (1024.0 * 1024.0)
        );
    }

    #[test]
    #[ignore = "需要交互式桌面（无桌面会话 / CI 上会失败）"]
    fn capture_desktop_smoke() {        let capture = WindowsCapture::new().unwrap();
        let shot = capture.capture_desktop().unwrap();

        assert_eq!(
            shot.frame.size(),
            shot.bounds.size(),
            "截图尺寸应等于虚拟桌面尺寸"
        );
        assert_eq!(
            shot.frame.pixel(0, 0)[3],
            255,
            "截图像素的 alpha 必须是不透明的"
        );
        assert!(!shot.monitors.is_empty());
        assert_eq!(
            shot.bounds,
            virtual_bounds(&shot.monitors),
            "DesktopShot::bounds 应等于各显示器的并集"
        );

        if let Some(cursor) = shot.cursor {
            assert!(
                shot.monitor_at(cursor).is_some(),
                "光标 {cursor:?} 应落在某台显示器上"
            );
            let local = shot.cursor_local().unwrap();
            assert_eq!(shot.to_global(local), cursor, "全局/局部坐标换算应往返一致");
        }

        // 局部区域捕获：坐标直接取全局物理像素，尺寸必须精确匹配
        let region = Rect::new(shot.bounds.x, shot.bounds.y, 32, 24);
        let frame = capture.capture_region(region).unwrap();
        assert_eq!(frame.size(), Size::new(32, 24));
        assert_eq!(frame.data().len(), 32 * 24 * 4);
        assert_eq!(
            frame.pixel(0, 0)[3],
            255,
            "区域截图的 alpha 也必须是不透明的"
        );

        // 真实桌面不可能是单色：如果 BitBlt 静默失败（DIB 全是 0），from_bgrx 之后
        // 整图会是纯黑不透明，这条断言正好能抓住那种退化情况。
        let mut colors = std::collections::HashSet::new();
        for y in (0..shot.frame.height() as i32).step_by(16) {
            for x in (0..shot.frame.width() as i32).step_by(16) {
                let px = shot.frame.pixel(x, y);
                assert_eq!(px[3], 255, "像素 ({x}, {y}) 的 alpha 应为 255");
                colors.insert(px);
            }
        }
        println!(
            "捕获证据：虚拟桌面 {:?}（{}x{} 像素），{} 台显示器，采样到 {} 种颜色，光标 {:?}",
            shot.bounds,
            shot.frame.width(),
            shot.frame.height(),
            shot.monitors.len(),
            colors.len(),
            shot.cursor
        );
        assert!(
            colors.len() > 1,
            "整屏采样只有一种颜色（{:?}），BitBlt 很可能没有真正抓到画面",
            colors
        );

        // 工具链自检：后端名 + 空区域报错
        assert_eq!(capture.backend_name(), "Windows GDI (BitBlt)");
        assert!(capture.capture_region(Rect::ZERO).is_err());
    }
}
