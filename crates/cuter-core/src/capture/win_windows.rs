//! 顶层窗口枚举（Windows 实现）——用于框选时的窗口边缘吸附。
//!
//! 这个模块只负责一件事：**按 z 序把当前桌面上所有可见的顶层窗口矩形列出来**。
//! 吸附算法是纯几何、在别处实现，所以这里不做任何吸附判断。
//!
//! `EnumWindows` 本身就按 z 序（最上层优先）从上到下回调，因此保持推入顺序即可。
//! 每个候选窗口要依次通过下面几关，**便宜的先做，跨进程的 DWM 查询放后面**：
//!
//! 1. 不可见（`IsWindowVisible`）与最小化（`IsIconic`）窗口跳过——最小化窗口的矩形
//!    是 `(-32000, -32000)` 这类垃圾值；
//! 2. 本进程自己的窗口跳过，否则遮罩 / 贴图 / 悬浮条会吸附到自己身上；
//! 3. DWM 幽灵窗口（UWP 应用挂起后残留的 cloaked 窗口）跳过；
//! 4. 矩形取 `DWMWA_EXTENDED_FRAME_BOUNDS`（肉眼看到的窗口边缘，不含不可见阴影），
//!    失败（老系统、非 DWM 合成）时回退 `GetWindowRect`；宽高必须都大于 0；
//! 5. 与虚拟桌面求交并**收敛为交集**——跨屏最大化窗口会远超桌面范围，
//!    留着会让吸附线跑到屏幕外；交集为空则跳过。
//!
//! 工具窗口（`WS_EX_TOOLWINDOW`）与无标题窗口仍然收录（`is_tool = true`），
//! 调用方会把它们排在后面，而不是直接丢掉。
//!
//! windows-rs 0.62 的坑（处理方式与 `capture/windows.rs` 保持一致）：
//!
//! * `BOOL` 是 `windows::core::BOOL`（`i32` 的 newtype），必须用 `.as_bool()` 判断；
//!   `TRUE` / `FALSE` 常量在 `Win32::Foundation`；
//! * 0.62 里带 `SetLastError` 的 BOOL API 大多已映射成 `windows_core::Result`——
//!   `EnumWindows`、`GetWindowRect`、`DwmGetWindowAttribute` 都是 `Result`，
//!   而 `IsWindowVisible` / `IsIconic` 仍返回裸 `BOOL`，两种写法不能混用；
//! * `Error::from_thread()` 把 `GetLastError()` 塞进了 FACILITY_WIN32
//!   （`0x8007_xxxx`），要剥掉高位才是真正的 Win32 错误码。
//!
//! 性能：本函数在每次框选开始时调用一次，但 `EnumWindows` 可能枚举到几百个窗口。
//! 过滤链保证了 DWM 查询只发生在真正的候选窗口上；标题用固定大小的栈缓冲区，
//! 类名只在 `RUST_LOG=debug` 时、且只对最上层若干个窗口查询，生产路径零额外开销。

use std::ffi::c_void;
use std::mem::size_of;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WS_EX_TOOLWINDOW,
};

use crate::capture::{SnapWindow, WindowDetector};
use crate::error::{Error, Result};
use crate::geom::Rect;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// 标题缓冲区的 UTF-16 码元数量。
///
/// 512 个 `u16`（1 KiB）足够容纳任何真实窗口标题，且是**栈上**数组：
/// 枚举几百个窗口也不会产生任何堆分配。
const TITLE_CAPACITY: usize = 512;

/// 类名缓冲区的 UTF-16 码元数量（Win32 类名上限 256，留一倍余量）。
const CLASS_NAME_CAPACITY: usize = 256;

/// 调试日志里最多打印多少个窗口的详情（类名等）。
///
/// 真实桌面常有几百个顶层窗口，逐个打印会让 `RUST_LOG=debug` 直接刷屏；
/// 这里只打印 z 序最靠前的若干个，其余只计入总数。
const DEBUG_WINDOW_LIMIT: usize = 8;

// ---------------------------------------------------------------------------
// 枚举器
// ---------------------------------------------------------------------------

/// Windows 平台的窗口枚举器。
///
/// 结构体刻意不带字段（与 `capture/windows.rs` 的 `WindowsCapture` 一致）：
/// 枚举所需的全部状态都在调用期间存在栈上，因此类型天然满足 `Send + Sync`，
/// 也不存在跨次调用会失效的缓存（显示器热插拔后不会给出陈旧的虚拟桌面范围）。
#[derive(Debug)]
pub struct WindowsWindowDetector;

impl WindowsWindowDetector {
    /// 创建枚举器；顺带确保进程已开启 DPI 感知（幂等）。
    ///
    /// 这一步不是可选的：`DWMWA_EXTENDED_FRAME_BOUNDS` 返回的**始终是物理像素**，
    /// 而 `GetSystemMetrics` / `GetWindowRect` 对 DPI 不感知的进程返回的是**被虚拟化的
    /// 坐标**（125% 缩放下 1920x1080 会变成 1536x864）。两者混用会让「与虚拟桌面求交」
    /// 按错误的尺度裁剪窗口矩形，吸附线随之偏移。开启 DPI 感知后两个来源统一为物理
    /// 像素，与本 crate「全局物理像素」的坐标约定一致。
    /// 重复调用安全、永不失败（已被 manifest 或更早的调用设定时只记一条 debug 日志）。
    #[must_use]
    pub fn new() -> Self {
        crate::capture::ensure_dpi_awareness();
        Self
    }
}

/// 与 [`WindowsWindowDetector::new`] 等价：绕开 `new` 的 `Default` 会让调用方拿到
/// 一个没设置 DPI 感知的枚举器，从而在缩放的显示器上产出错位的矩形。
impl Default for WindowsWindowDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowDetector for WindowsWindowDetector {
    fn visible_windows(&self) -> Result<Vec<SnapWindow>> {
        // 枚举期间的全部状态都留在栈上，调用结束后不保留任何可能过期的缓存。
        let mut out: Vec<SnapWindow> = Vec::new();
        let started = Instant::now();

        // SAFETY: 回调签名与 WNDENUMPROC 一致；lparam 指向本函数栈上的 out，
        // 它在整个枚举期间存活（既未移动也未重新赋值，用 `&raw mut` 取地址不产生
        // 临时引用），EnumWindows 在当前线程串行调用回调，没有其它线程访问它。
        let outcome =
            unsafe { EnumWindows(Some(enum_window_proc), LPARAM((&raw mut out) as isize)) };
        if let Err(err) = outcome {
            return Err(win32_error("EnumWindows", &err));
        }

        log::debug!(
            "顶层窗口枚举：{} 个候选窗口，耗时 {:?}",
            out.len(),
            started.elapsed()
        );
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// 枚举回调
// ---------------------------------------------------------------------------

/// `EnumWindows` 的回调：把当前 `hwnd` 转成 [`SnapWindow`] 追加进 `lparam` 指向的 `Vec`。
///
/// # Safety
///
/// `lparam` 必须是 [`WindowsWindowDetector::visible_windows`] 传入的
/// `*mut Vec<SnapWindow>`，该 `Vec` 在整个枚举期间存活且可写，
/// 并且枚举期间没有其它线程访问它（`EnumWindows` 串行调用本回调）。
unsafe extern "system" fn enum_window_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: 见函数文档——lparam 指向一个存活且独占的 Vec<SnapWindow>。
    let out = unsafe { &mut *(lparam.0 as *mut Vec<SnapWindow>) };
    // 当前 Vec 的长度恰好就是该窗口在 z 序里的位置（只用于调试日志）。
    let z_index = out.len();
    if let Some(window) = snap_window_for(hwnd, z_index) {
        out.push(window);
    }
    // 恒返回 TRUE：单个窗口判断失败绝不能中断枚举（返回 FALSE 会让 EnumWindows 报错）。
    TRUE
}

/// 判断单个顶层窗口是否参与吸附；`Some` 表示收录。
///
/// `z_index` 只用于调试日志（它在 z 序里的位置），不参与任何判断。
fn snap_window_for(hwnd: HWND, z_index: usize) -> Option<SnapWindow> {
    // 1) 可见性与最小化：最便宜的两个判断，先挡掉绝大多数系统窗口。
    // SAFETY: hwnd 由 EnumWindows 提供，在本次回调期间一直有效；两个查询都是只读的。
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }
    // SAFETY: 同上。
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return None;
    }

    // 2) 本进程自己的窗口（遮罩、贴图、悬浮工具条……）不参与吸附。
    let mut pid = 0u32;
    // SAFETY: hwnd 有效；`&raw mut pid` 指向本函数栈上存活的可写 u32，
    // 该指针只在调用期间被使用，且 GetWindowThreadProcessId 必定写入它。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    // SAFETY: GetCurrentProcessId 无参数、无副作用，只读当前进程 id。
    if pid == unsafe { GetCurrentProcessId() } {
        return None;
    }

    // 3) DWM 幽灵窗口：UWP 应用挂起后窗口仍然「可见」，但已被 DWM 隐藏（cloaked），
    //    它的矩形通常还停在挂起前的位置，会误导吸附。
    let mut cloaked = 0u32;
    // SAFETY: cloaked 是有效的可写 u32，字节数由 size_of 精确给出，
    // 该属性（DWMWA_CLOAKED）要求的正是一个 u32。
    let cloaked_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            size_of::<u32>() as u32,
        )
    }
    .is_ok();
    // 查询失败（Win7 没有这个属性、非 DWM 合成、窗口已被销毁）一律按「未 cloaked」
    // 处理：宁可多收一个，也不要因为一次查询失败漏掉用户看得见的窗口。
    if cloaked_ok && cloaked != 0 {
        return None;
    }

    // 4) 矩形：优先 DWM 扩展边框，宽高必须都大于 0。
    let raw = window_rect(hwnd)?;
    let bounds = rect_from_ltrb(raw.left, raw.top, raw.right, raw.bottom);
    if bounds.is_empty() {
        return None;
    }

    // 5) 收敛到虚拟桌面：跨屏最大化窗口的边框会远超桌面范围，
    //    留在 bounds 里会让吸附线跑到屏幕之外。
    let bounds = clamp_to_desktop(bounds, virtual_desktop_rect())?;

    // 6) 标题与扩展样式。标题走固定栈缓冲区，没有堆分配。
    let mut title_buf = [0u16; TITLE_CAPACITY];
    let title = window_text(hwnd, &mut title_buf);
    // SAFETY: hwnd 有效；GWL_EXSTYLE 是只读查询，失败时返回 0（即「无扩展样式」）。
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let window = SnapWindow {
        bounds,
        is_tool: is_tool_like(ex_style, &title),
        title,
    };

    // 类名对调试吸附问题很有用，但不在 SnapWindow 里（不为了日志污染公开结构），
    // 因此只在 debug 日志开启时、且只对最上层若干个窗口查询，避免刷屏与额外开销。
    if log::log_enabled!(log::Level::Debug) && z_index < DEBUG_WINDOW_LIMIT {
        log::debug!(
            "顶层窗口 #{z_index}：类名「{}」标题「{}」矩形 {:?} 工具窗口 {}",
            window_class(hwnd),
            window.title,
            window.bounds,
            window.is_tool
        );
    }

    Some(window)
}

// ---------------------------------------------------------------------------
// Win32 查询
// ---------------------------------------------------------------------------

/// 窗口矩形：优先 `DWMWA_EXTENDED_FRAME_BOUNDS`，失败时回退 `GetWindowRect`。
///
/// 前者是**肉眼看到的窗口边缘**，不含 DWM 为窗口绘制的不可见阴影——用
/// `GetWindowRect` 会让吸附线离窗口边缘差出几个像素的阴影宽度。
/// 没有 DWM 属性可用时（老系统、非合成窗口）只能退回后者，略有偏差但不致失败。
///
/// 注意两者的坐标尺度不同：DWM 属性返回的**始终是物理像素**，而 `GetWindowRect` 对
/// DPI 不感知的进程返回被虚拟化的坐标。因此本模块依赖 [`WindowsWindowDetector::new`]
/// 设置的 DPI 感知来保证两个来源（以及 `GetSystemMetrics`）落在同一坐标系里。
fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: rect 是有效的可写 RECT，字节数由 size_of 精确给出；
    // hwnd 由 EnumWindows 提供，在回调期间有效。
    let dwm = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast::<c_void>(),
            size_of::<RECT>() as u32,
        )
    };
    if dwm.is_ok() {
        return Some(rect);
    }

    // SAFETY: hwnd 在回调期间有效；rect 是有效的可写 RECT。
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.ok()?;
    Some(rect)
}

/// 窗口标题；读取失败或没有标题时返回空字符串。
///
/// 对**其它进程**的窗口，`GetWindowTextW` 只取标题栏文本、不会向对方发送
/// `WM_GETTEXT`（MSDN 明确说明的行为），所以目标进程无响应时这里也不会卡住。
fn window_text(hwnd: HWND, buf: &mut [u16]) -> String {
    // SAFETY: hwnd 有效；buf 是可写切片，其长度会作为 nMaxCount 传给系统。
    let len = unsafe { GetWindowTextW(hwnd, buf) };
    utf16_prefix(buf, len)
}

/// 窗口类名（仅用于调试日志）。
fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; CLASS_NAME_CAPACITY];
    // SAFETY: hwnd 有效；buf 是可写切片，其长度会作为 nMaxCount 传给系统。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    utf16_prefix(&buf, len)
}

/// 虚拟桌面矩形（`GetSystemMetrics` 的四个虚拟屏幕指标）。
fn virtual_desktop_rect() -> Rect {
    // SAFETY: GetSystemMetrics 只读会话指标，无参数、无副作用。
    let (x, y, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    desktop_rect_from_metrics(x, y, w, h)
}

// ---------------------------------------------------------------------------
// 纯函数（可无头测试）
// ---------------------------------------------------------------------------

/// 由「左、上、右、下」四个坐标构造 [`Rect`]。
///
/// 与 [`Rect::from_ltrb`] 一样把右下角当作**开区间**（`right` / `bottom` 指向最后一个
/// 像素之外），这正是 `RECT` 的约定，因此 Win32 矩形不需要 ±1 修正。
/// 负坐标原样保留（多屏布局里最左侧显示器的 `x` 是负数），
/// `right <= left` 或 `bottom <= top` 的非法区间退化成空矩形。
fn rect_from_ltrb(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::from_ltrb(left, top, right, bottom)
}

/// 虚拟桌面矩形：`GetSystemMetrics` 的 (x, y, 宽, 高) 四个指标 → [`Rect`]。
///
/// 对 DPI 感知的进程（[`WindowsWindowDetector::new`] 保证）指标是**物理像素**，
/// 最左侧显示器在主屏左边时 `x` 为负。指标异常（宽高为 0，例如无桌面会话）会得到
/// 空矩形，调用方据此跳过全部窗口而不是崩溃。
fn desktop_rect_from_metrics(x: i32, y: i32, w: i32, h: i32) -> Rect {
    // 这是来自系统的输入，用饱和加法避免极端值在 debug 构建下 panic。
    rect_from_ltrb(x, y, x.saturating_add(w), y.saturating_add(h))
}

/// 把窗口矩形裁剪到虚拟桌面以内；`None` 表示与桌面完全没有交集（或本身就是空矩形）。
///
/// 跨屏最大化的窗口会把矩形撑到桌面之外，裁剪既让吸附线不越界，
/// 也让后续「与桌面有交集」这条不变量对调用方恒成立。
fn clamp_to_desktop(bounds: Rect, desktop: Rect) -> Option<Rect> {
    if bounds.is_empty() || desktop.is_empty() {
        return None;
    }
    let clipped = bounds.intersect(desktop);
    if clipped.is_empty() {
        None
    } else {
        Some(clipped)
    }
}

/// 是否属于「应排在后面」的窗口：工具窗口，或没有标题的窗口。
///
/// 这两类窗口仍然参与吸附（很多 HUD / 无边框浮层其实是用户想对齐的目标），
/// 只是排序时优先级更低，因此这里用「或」而不是「与」。
fn is_tool_like(ex_style: u32, title: &str) -> bool {
    ex_style & WS_EX_TOOLWINDOW.0 != 0 || title.is_empty()
}

/// Win32 写进缓冲区的 UTF-16 文本 → `String`。
///
/// `len` 是 API 的返回值（已写入的码元数，不含结尾的 0，失败时为 0 或负数）。
/// 末尾可能已经有系统写入的 0，也可能因为缓冲区被填满而没有 0，这里统一在第一个 0
/// 处截断，保证结果里不可能出现裸 `\0`；非法代理项由 `from_utf16_lossy` 替换。
fn utf16_prefix(buf: &[u16], len: i32) -> String {
    if len <= 0 {
        return String::new();
    }
    let text = &buf[..(len as usize).min(buf.len())];
    let end = text
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

// ---------------------------------------------------------------------------
// windows-rs 错误码转换
// ---------------------------------------------------------------------------

/// 把 windows-rs 的错误转成本 crate 的 [`Error::Win32`]（错误码已还原为原始 Win32 码）。
fn win32_error(api: &'static str, err: &windows::core::Error) -> Error {
    Error::Win32 {
        api,
        code: win32_code(err),
    }
}

/// 从 [`windows::core::Error`] 还原原始 Win32 错误码。
///
/// 生成的绑定把失败的 `BOOL` 变成 `Error::from_thread()`，其中 `GetLastError()` 的值被
/// `HRESULT::from_win32` 塞进了 FACILITY_WIN32（`0x8007_xxxx`）；直接打印 HRESULT 会和
/// MSDN 上的错误码对不上，所以这里把高位剥掉。
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
    use std::time::Duration;
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;

    // -----------------------------------------------------------------------
    // 纯函数
    // -----------------------------------------------------------------------

    #[test]
    fn rect_from_ltrb_supports_negative_origin() {
        // 多屏布局里最左侧显示器在主屏左边，x / y 都可能为负。
        assert_eq!(
            rect_from_ltrb(-1920, -100, 0, 980),
            Rect::new(-1920, -100, 1920, 1080)
        );
        assert_eq!(
            rect_from_ltrb(-100, -50, -90, -40),
            Rect::new(-100, -50, 10, 10)
        );
    }

    #[test]
    fn rect_from_ltrb_keeps_half_open_right_bottom() {
        let rect = rect_from_ltrb(0, 0, 1920, 1080);
        assert_eq!((rect.right(), rect.bottom()), (1920, 1080));
        assert_eq!(rect.size(), crate::geom::Size::new(1920, 1080));
        assert_eq!(rect, Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn rect_from_ltrb_degenerates_on_zero_or_inverted_span() {
        // 零宽：RECT 的 left == right 是合法输入，必须退化成空矩形而不是取绝对值
        assert!(rect_from_ltrb(10, 10, 10, 20).is_empty());
        assert_eq!(rect_from_ltrb(10, 10, 10, 20).w, 0);
        // 零高
        assert!(rect_from_ltrb(10, 10, 20, 10).is_empty());
        // 完全反向：宽高收敛为 0，但**不是** Rect::ZERO——左上角仍保留原始的 left/top
        assert!(rect_from_ltrb(20, 20, 10, 10).is_empty());
        assert_eq!(
            rect_from_ltrb(20, 20, 10, 10).origin(),
            crate::geom::Point::new(20, 20)
        );
        assert_eq!(
            (
                rect_from_ltrb(20, 20, 10, 10).w,
                rect_from_ltrb(20, 20, 10, 10).h
            ),
            (0, 0)
        );
        // 零尺寸的退化矩形才等于 Rect::ZERO
        assert_eq!(rect_from_ltrb(0, 0, 0, 0), Rect::ZERO);
        // 退化矩形的左上角仍保留原坐标（调用方据此判断它在哪里）
        assert_eq!(
            rect_from_ltrb(10, 10, 10, 10).origin(),
            crate::geom::Point::new(10, 10)
        );
    }

    #[test]
    fn desktop_rect_from_metrics_handles_negative_origin() {
        // 主屏 2560x1440 右侧接一台 1920x1080，虚拟桌面从 (-1920, 0) 开始
        assert_eq!(
            desktop_rect_from_metrics(-1920, 0, 4480, 1440),
            Rect::new(-1920, 0, 4480, 1440)
        );
        assert_eq!(
            desktop_rect_from_metrics(0, 0, 1920, 1080),
            Rect::new(0, 0, 1920, 1080)
        );
    }

    #[test]
    fn desktop_rect_from_metrics_tolerates_bad_values() {
        // 无桌面会话 / 指标不可用时是 0，必须得到空矩形而不是 panic
        assert!(desktop_rect_from_metrics(0, 0, 0, 0).is_empty());
        assert!(desktop_rect_from_metrics(0, 0, 100, 0).is_empty());
        // 指标极端值不能触发 i32 溢出
        let huge = desktop_rect_from_metrics(i32::MAX - 10, i32::MIN, 100, 100);
        assert_eq!(huge.w, 10);
        assert!(!huge.is_empty());
    }

    #[test]
    fn clamp_to_desktop_keeps_fully_inside_rect_untouched() {
        let desktop = Rect::new(0, 0, 1920, 1080);
        let inside = Rect::new(100, 100, 200, 150);
        assert_eq!(clamp_to_desktop(inside, desktop), Some(inside));
        // 贴着右下角（右/下开区间）也算完全在内
        assert_eq!(
            clamp_to_desktop(Rect::new(1820, 980, 100, 100), desktop),
            Some(Rect::new(1820, 980, 100, 100))
        );
    }

    #[test]
    fn clamp_to_desktop_crops_partial_overflow() {
        let desktop = Rect::new(0, 0, 1920, 1080);
        // 左侧与上方越界（多屏负坐标 / 跨屏最大化）
        assert_eq!(
            clamp_to_desktop(Rect::new(-50, -50, 200, 200), desktop),
            Some(Rect::new(0, 0, 150, 150))
        );
        // 右侧与下方越界
        assert_eq!(
            clamp_to_desktop(Rect::new(1900, 1060, 100, 100), desktop),
            Some(Rect::new(1900, 1060, 20, 20))
        );
        // 负原点的虚拟桌面同样成立
        let wide = Rect::new(-1920, 0, 4480, 1440);
        assert_eq!(
            clamp_to_desktop(Rect::new(-2000, -10, 4000, 2000), wide),
            Some(Rect::new(-1920, 0, 3920, 1440))
        );
    }

    #[test]
    fn clamp_to_desktop_rejects_rect_fully_outside() {
        let desktop = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            clamp_to_desktop(Rect::new(2000, 100, 100, 100), desktop),
            None
        );
        assert_eq!(
            clamp_to_desktop(Rect::new(-100, -100, 50, 50), desktop),
            None
        );
        // 仅在边上相接（无面积交集）也算完全在外
        assert_eq!(
            clamp_to_desktop(Rect::new(1920, 0, 100, 100), desktop),
            None
        );
        assert_eq!(
            clamp_to_desktop(Rect::new(0, 1080, 100, 100), desktop),
            None
        );
    }

    #[test]
    fn clamp_to_desktop_rejects_zero_sized_inputs() {
        let desktop = Rect::new(0, 0, 1920, 1080);
        assert_eq!(clamp_to_desktop(Rect::new(10, 10, 0, 100), desktop), None);
        assert_eq!(clamp_to_desktop(Rect::new(10, 10, 100, 0), desktop), None);
        assert_eq!(clamp_to_desktop(Rect::ZERO, desktop), None);
        // 桌面本身为空（无桌面会话）时一切都被拒绝
        assert_eq!(clamp_to_desktop(Rect::new(0, 0, 10, 10), Rect::ZERO), None);
    }

    #[test]
    fn is_tool_like_covers_all_style_title_combinations() {
        let tool = WS_EX_TOOLWINDOW.0;
        // 只有 WS_EX_TOOLWINDOW
        assert!(is_tool_like(tool, "记事本"));
        // 只有空标题
        assert!(is_tool_like(0, ""));
        // 两者都有
        assert!(is_tool_like(tool, ""));
        // 两者都无
        assert!(!is_tool_like(0, "记事本"));
        assert!(!is_tool_like(0x0004_0000, "Cuter")); // WS_EX_APPWINDOW
        // 必须按「位」判断，不能按字节/子串判断
        assert!(!is_tool_like(0x0000_0100, "Cuter"));
        // 与 WS_EX_TOOLWINDOW 叠加的其它样式仍然算工具窗口
        assert!(is_tool_like(tool | 0x0000_0008, "Cuter")); // WS_EX_TOPMOST
    }

    #[test]
    fn utf16_prefix_stops_at_nul_and_ignores_bad_lengths() {
        let mut buf = [0u16; 8];
        for (slot, ch) in buf.iter_mut().zip("Cuter".encode_utf16()) {
            *slot = ch;
        }
        // 系统返回值不含结尾 0
        assert_eq!(utf16_prefix(&buf, 5), "Cuter");
        // 长度超过缓冲区（不应 panic）时按缓冲区长度处理
        assert_eq!(utf16_prefix(&buf, 99), "Cuter");
        // 0 / 负数表示失败或空标题
        assert_eq!(utf16_prefix(&buf, 0), "");
        assert_eq!(utf16_prefix(&buf, -1), "");
        // 中间出现 0 时在 0 处截断，结果里不会残留裸 \0
        let embedded = [b'A' as u16, 0, b'B' as u16];
        assert_eq!(utf16_prefix(&embedded, 3), "A");
        // 缓冲区被填满且没有结尾 0
        assert_eq!(utf16_prefix(&[b'X' as u16; 4], 4), "XXXX");
        assert_eq!(utf16_prefix(&[], 0), "");
    }

    #[test]
    fn win32_code_unwraps_facility_win32_hresult() {
        let access_denied = windows::core::Error::from_hresult(windows::core::HRESULT(
            (0x8007_0000u32 | ERROR_ACCESS_DENIED.0) as i32,
        ));
        assert_eq!(win32_code(&access_denied), ERROR_ACCESS_DENIED.0);
        let message = win32_error("EnumWindows", &access_denied).to_string();
        assert!(
            message.contains("错误码 5"),
            "错误信息应带原始 Win32 错误码，实际：{message}"
        );
    }

    // -----------------------------------------------------------------------
    // 真实 Win32 调用
    // -----------------------------------------------------------------------

    #[test]
    fn visible_windows_does_not_error() {
        let detector = WindowsWindowDetector::new();
        let windows = detector.visible_windows().expect("枚举顶层窗口不应失败");
        let desktop = virtual_desktop_rect();
        assert!(
            !desktop.is_empty(),
            "虚拟桌面矩形不应为空（无桌面会话时这条断言会失败）：{desktop:?}"
        );

        // 不断言数量：不同机器差异极大（可能只有几个窗口，也可能几百个）。
        for w in &windows {
            assert!(
                w.bounds.w > 0 && w.bounds.h > 0,
                "窗口矩形必须非空：{} @ {:?}",
                w.title,
                w.bounds
            );
            assert!(
                w.bounds.intersects(desktop),
                "窗口 {:?} 与虚拟桌面 {desktop:?} 应有交集",
                w.bounds
            );
            assert!(
                desktop.contains_rect(w.bounds),
                "窗口 {:?} 应已被收敛到虚拟桌面 {desktop:?} 以内",
                w.bounds
            );
            assert!(!w.title.contains('\0'), "标题里出现了裸 NUL：{:?}", w.title);
        }
        println!(
            "枚举到 {} 个候选窗口（虚拟桌面 {desktop:?}，未断言数量）",
            windows.len()
        );
    }

    #[test]
    #[ignore = "需要交互式桌面"]
    fn visible_windows_lists_something() {
        let detector = WindowsWindowDetector::new();
        let windows = detector.visible_windows().expect("枚举顶层窗口不应失败");
        let desktop = virtual_desktop_rect();
        println!("虚拟桌面：{desktop:?}");

        println!("枚举到 {} 个候选窗口，最上层 5 个：", windows.len());
        for (i, w) in windows.iter().take(5).enumerate() {
            println!(
                "  #{i} 标题「{}」矩形 {:?}（{}x{}）工具窗口 {}",
                w.title, w.bounds, w.bounds.w, w.bounds.h, w.is_tool
            );
        }
        assert!(!windows.is_empty(), "交互式桌面上至少应枚举到 1 个顶层窗口");

        // 连续多次计时：第 1 次可能包含 dwmapi.dll 的惰性加载，取最小值作为稳态耗时。
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let started = Instant::now();
            let _ = detector.visible_windows().expect("重复枚举不应失败");
            best = best.min(started.elapsed());
        }
        println!(
            "稳态枚举耗时（{} 个窗口）：{best:?}（目标：200 窗口 < 5 ms）",
            windows.len()
        );
        assert!(
            best.as_millis() < 50,
            "枚举耗时 {best:?} 远超预期（200 窗口的目标是 < 5 ms）"
        );
    }
}
