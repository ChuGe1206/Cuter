//! 桌面集成：全局热键、托盘图标、剪贴板。
//!
//! 这一层是唯一直接接触「操作系统外壳」的地方，`cuter-core` / `cuter-ui` 都不知道
//! 托盘和热键的存在，方便将来替换成别的壳（例如 Linux 的 StatusNotifier）。
//!
//! ## 为什么热键与托盘走「事件处理器」而不是每帧轮询
//!
//! 早先 `logic()` 里有无条件的 `request_repaint_after(120ms)` 兜底（那时靠轮询），
//! 但**一次热键最坏要等 120 ms 才被读到**——对一个"按下去就要立刻看到遮罩"的工具
//! 来说这是能明显感觉到的迟钝。
//!
//! `global-hotkey` / `tray-icon` / `muda` 都提供 `set_event_handler`：事件到达的瞬间
//! 就在它们自己的线程上回调，于是可以立刻 `ctx.request_repaint()` 把 UI 唤醒。
//!
//! ⚠️ **代价是二选一**：官方文档写明设了 handler 之后 `receiver()` 收不到任何事件
//! （事件被转给 handler）。所以这里自己建一条 `mpsc` 队列：handler 填、`poll` 读。
//!
//! ### 实测（`tools/probe/perf_probe.ps1`，release 产物、托盘模式）
//!
//! ```text
//! Alt+C -> 遮罩可见：min 82 / median 117 / max 117 ms（3 次）
//! app 侧阶段日志：请求登记 -> 开始捕获 = +0 ms（同一帧）
//!                 捕获 + 合成光标 = 34~54 ms
//! ```
//!
//! 也就是说**请求一被读到就是同帧开始捕获**，剩下的 30~70 ms 全在"多久才被读到"。
//! 这一段的量级不由本模块决定：托盘模式下我们自己的窗口全是隐藏的，而 eframe 把
//! **不可见窗口**的重绘请求统一推迟到 `now + 100 ms`
//! （`eframe::native::run::INVISIBLE_WINDOW_REPAINT_INTERVAL`），
//! 那条心跳已经删掉（见 HANDOFF 第 35 节：它让托盘待命时白烧 0.7% GPU），
//! 现在完全靠下面这套"事件线程立刻叫醒 UI"的机制，待命时一帧都不画。
//! 想再往下压就得绕开这条节流（例如让根窗口"可见但停在屏幕外"），
//! 那是另一档的事，见 `docs/HANDOFF.md` 第 14 节。

use cuter_ui::{fill, tr};

use std::borrow::Cow;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use cuter_core::config::HotkeyConfig;
use cuter_core::frame::Frame;

// ---------------------------------------------------------------------------
// 唤醒 UI 线程
// ---------------------------------------------------------------------------

/// 把 UI 线程从「窗口不可见」的重绘节流里**立刻**叫醒。
///
/// 为什么需要它：托盘模式下我们自己的窗口全是隐藏的，而 eframe 把隐藏窗口的重绘请求
/// 统一推迟到 `now + 100 ms`
/// （`eframe::native::run::INVISIBLE_WINDOW_REPAINT_INTERVAL`，为的是隐藏窗口别空转烧
/// CPU）。于是光靠 [`egui::Context::request_repaint`]，一次热键最坏要等 100 ms 才开始
/// 被处理，实测「Alt+C → 遮罩可见」因此卡在 119~220 ms（见 `docs/HANDOFF.md` 第 15 节）。
///
/// 绕开的办法是**替隐藏的根窗口投递一条 `WM_PAINT`**：
///
/// * `winit` 的 `WM_PAINT` 分支（`platform_impl/windows/event_loop.rs`）会**直接派发**
///   `RedrawRequested` —— 是"派发事件"，不是"请求重绘"，所以不经过那条节流；
/// * `eframe` 收到 `RedrawRequested` 就直接 `run_ui_and_paint`，`logic()` 立刻跑；
/// * 隐藏窗口当然不会自己收到 `WM_PAINT`（这正是 eframe 要加节流的原因），
///   但**别人替它投递的消息照常派发**。
///
/// 安全边界：
///
/// * 句柄来自 eframe 的 `Frame`（它持有的是**根窗口**的句柄）；拿不到就静默退化成
///   普通的 `request_repaint()` —— 这条路径只是优化，不是正确性依赖；
/// * 只在用户动作（热键 / 托盘）时投递，不会自激成重绘风暴；投递本身没有副作用。
#[derive(Clone, Default)]
pub struct UiWake {
    /// 根窗口句柄（`HWND`），0 表示还没拿到。
    hwnd: Arc<AtomicIsize>,
    /// 最近一次被唤醒的时刻。热键线程写、UI 线程取走，只用来打一条"唤醒 → 被处理"
    /// 的耗时日志——热键链路里唯一无法从 app 侧量到的就是这一段。
    last_wake: Arc<Mutex<Option<std::time::Instant>>>,
}

/// 遮罩窗被换掉的窗口过程（原过程的地址）。
#[cfg(windows)]
static OVERLAY_OLD_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 遮罩窗自己的窗口过程：只改 `WM_NCHITTEST`，其余原样转给 winit 的过程。
#[cfg(windows)]
unsafe extern "system" fn overlay_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{CallWindowProcW, HTCLIENT, WM_NCHITTEST};

    if msg == WM_NCHITTEST {
        // 遮罩铺满整块屏幕，"在不在窗口里"的答案永远是在。
        return windows::Win32::Foundation::LRESULT(HTCLIENT as isize);
    }
    let old = OVERLAY_OLD_PROC.load(std::sync::atomic::Ordering::Relaxed);
    if old == 0 {
        return windows::Win32::Foundation::LRESULT(0);
    }
    // SAFETY: 这个地址是 winit 装进来的窗口过程，签名与 `WNDPROC` 一致。
    type RawProc = unsafe extern "system" fn(
        windows::Win32::Foundation::HWND,
        u32,
        windows::Win32::Foundation::WPARAM,
        windows::Win32::Foundation::LPARAM,
    ) -> windows::Win32::Foundation::LRESULT;
    let raw: RawProc = unsafe { std::mem::transmute::<isize, RawProc>(old) };
    let proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC = Some(raw);
    // SAFETY: 参数是系统交给我们的，原样转交。
    unsafe { CallWindowProcW(proc, hwnd, msg, wparam, lparam) }
}

/// 抢过某个窗口的命中判定：`WM_NCHITTEST` 一律回 `HTCLIENT`。
///
/// 为什么需要：winit 建出来的遮罩窗客户区**比窗口矮一行**（实测 `GetClientRect`
/// = 1920×1080、客户区原点却在 (0,1)），于是屏幕上最上面那一行像素落在客户区之外。
/// `DefWindowProc` 把它判成标题栏（`WS_CAPTION` 在时就是 `HTCAPTION`）：在那一行按下
/// 再拖，Windows 自己进"拖标题栏"模态循环，整块冻结帧跟着鼠标走——用户反馈的
/// "鼠标移动到屏幕边缘左键移动，就会把屏幕帧移动"，而且只有屏幕上沿和两个上角会这样。
///
/// 单纯摘 `WS_CAPTION` 不行：命中判定会变成 `HTNOWHERE`，鼠标事件干脆掉到下面那个窗口去。
/// 所以不动样式，只换窗口过程。返回是否真的换过（窗口还没建出来时是 false）。
#[cfg(windows)]
fn own_hit_test(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GWL_WNDPROC, GetWindowLongPtrW, SetWindowLongPtrW};

    // SAFETY: 只查询/改写这个窗口的窗口过程，句柄来自我们自己的窗口枚举。
    let old = unsafe { GetWindowLongPtrW(hwnd, GWL_WNDPROC) };
    // 函数项先取裸指针再转 `isize`：直接 `as isize` 会触发 `function_casts_as_integer`。
    let ours = overlay_wndproc as *const () as isize;
    if old == 0 || old == ours {
        return false;
    }
    OVERLAY_OLD_PROC.store(old, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: 新过程是本模块里的静态函数，签名与 `WNDPROC` 一致，且进程内一直有效。
    unsafe { SetWindowLongPtrW(hwnd, GWL_WNDPROC, ours) };
    true
}
impl UiWake {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 根窗口句柄是否已经拿到（没拿到就不必每帧再问 eframe 一次）。
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.hwnd.load(Ordering::Relaxed) != 0
    }

    /// 取走「上一次唤醒到现在」的毫秒数（没有唤醒过则返回 `None`）。
    #[must_use]
    pub fn take_wake_age_ms(&self) -> Option<u128> {
        let mut slot = self
            .last_wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.take().map(|at| at.elapsed().as_millis())
    }

    /// 根窗口现在是否还显示在屏幕上（`IsWindowVisible`）。
    ///
    /// 截图前"等自己的窗口消失"就靠它：`ViewportCommand::Visible(false)` 从下发到
    /// 真的 `ShowWindow(SW_HIDE)` 之间隔着 eframe → winit → 消息循环一整条链路，
    /// 实测本机要 **约 110 ms**（见 `docs/HANDOFF.md` 第 15 节）。**猜一个时长是不行的**：
    /// 猜短了就把自己的窗口截进图里（`tools/probe/hide_own_windows_e2e.ps1` 抓到的
    /// `dev=16.9` 就是这么来的），猜长了就白等。
    ///
    /// 拿不到句柄时返回 `false`（当作"已经看不见"），于是调用方退回按时间等。
    #[must_use]
    pub fn root_is_visible(&self) -> bool {
        #[cfg(windows)]
        {
            let raw = self.hwnd.load(Ordering::Relaxed);
            if raw == 0 {
                return false;
            }
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
            // SAFETY: 句柄来自 eframe 的根窗口；`IsWindowVisible` 只是查询一个窗口样式位，
            // 句柄失效时返回 0（FALSE），不会出问题。
            unsafe { IsWindowVisible(HWND(raw as *mut core::ffi::c_void)).as_bool() }
        }
        #[cfg(not(windows))]
        false
    }

    /// 我们自己是否有**标题匹配**的顶层窗口正显示在屏幕上。
    ///
    /// 和 [`UiWake::root_is_visible`] 是两件事：那个只认根窗口（设置窗口），
    /// 而遮罩、贴图都是**子视口**，各有自己的 HWND。第一版的"截图前等自己窗口消失"
    /// 只问了根窗口，于是「遮罩里点重新截图」那条路以为自己没有窗口在屏幕上，
    /// 直接抓图——**把遮罩自己冻进了新的一帧**（现象是整屏变成遮罩的底色，
    /// 见 `docs/HANDOFF.md` 第 20 节）。
    ///
    /// 句柄拿不到、枚举失败时返回 `false`（当作"没有"），调用方仍按时间兜底。
    #[must_use]
    pub fn window_with_title_visible(title: &str) -> bool {
        #[cfg(windows)]
        {
            Self::find_visible_window(title).is_some()
        }
        #[cfg(not(windows))]
        {
            let _ = title;
            false
        }
    }

    /// 标题匹配的顶层窗口是否**真的挡在桌面上**（可见 **且** 与虚拟桌面有交集）。
    ///
    /// 比 [`UiWake::window_with_title_visible`] 更严：遮罩视口为了复用窗口（省掉每次
    /// 截图 ~200 ms 的建窗口开销，见 HANDOFF 第 24 节）平时会**停在桌面之外**
    /// （位置 −32000、尺寸 1×1）。这种窗口 `IsWindowVisible` 仍然是真，
    /// 于是"截图前等自己窗口消失"会以为遮罩还挡着屏幕，白等满 900 ms 超时。
    ///
    /// 所以凡是用来判断"我自己的窗口会不会被截进图里"的地方，都必须用这个版本。
    #[must_use]
    pub fn window_with_title_on_desktop(title: &str) -> bool {
        #[cfg(windows)]
        {
            Self::find_visible_window(title).is_some_and(Self::window_intersects_desktop)
        }
        #[cfg(not(windows))]
        {
            let _ = title;
            false
        }
    }

    /// 把标题匹配的那个窗口的命中去全部抢过来：`WM_NCHITTEST` 一律回 `HTCLIENT`。
    ///
    /// 为什么必须这么干：遮罩窗铺满屏幕，但 winit 建出来的窗口客户区**比窗口矮一行**
    /// （实测 `GetClientRect` = 1920×1080 而客户区原点在 (0,1)）。于是屏幕上最上面的
    /// 一行像素落在客户区之外，`DefWindowProc` 会把它判成标题栏（带 `WS_CAPTION` 时是
    /// `HTCAPTION`）：用户在那一行按下并拖动，Windows 直接进"拖标题栏"模态循环，
    /// 整块冻结帧跟着鼠标走——"鼠标移动到屏幕边缘左键移动，就会把屏幕帧移动"
    /// （用户原话；只有屏幕上沿和两个上角会这样，因为那几个点正是 `HTCAPTION`）。
    ///
    /// 连样式一起摘会变成 `HTNOWHERE`（鼠标事件干脆掉到下面那个窗口去），所以这里
    /// 不动样式，只把窗口过程换成自己的：`WM_NCHITTEST` 回 `HTCLIENT`，其余转给原过程。
    ///
    /// 窗口还没建出来时返回 `false`，调用方留着下一帧再试。
    #[must_use]
    pub fn take_over_hit_test(title: &str) -> bool {
        #[cfg(windows)]
        {
            Self::find_visible_window(title).is_some_and(own_hit_test)
        }
        #[cfg(not(windows))]
        {
            let _ = title;
            false
        }
    }

    /// 在本进程的顶层窗口里找标题匹配且**可见**的那一个。
///
/// 只按标题 + 进程 id 匹配：我们自己的窗口标题是自己定的（"Cuter 截图" / "Cuter 贴图" / …），
/// 不会和别的程序撞。枚举顶层窗口在真实桌面上约 1 ms（见 HANDOFF 第 7 节），
/// 而它只在"截图前等自己窗口消失"这条路上被调用，代价可以接受。
#[cfg(windows)]
fn find_visible_window(title: &str) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows::core::BOOL;

    struct Search<'a> {
        title: &'a str,
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam 是调用方传进来的 `&mut Search`，在整个枚举期间都活着。
        let search = unsafe { &mut *(lparam.0 as *mut Search<'_>) };
        let mut pid = 0u32;
        // SAFETY: 只是查询这个窗口的进程 id。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != search.pid {
            return BOOL(1);
        }
        // SAFETY: 窗口存活时查询可见性是安全的。
        if !unsafe { IsWindowVisible(hwnd).as_bool() } {
            return BOOL(1);
        }
        // SAFETY: 先问长度再取文本，缓冲区按长度分配。
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return BOOL(1);
        }
        let mut buffer = vec![0u16; len as usize + 1];
        // SAFETY: 缓冲区长度是 len + 1，GetWindowTextW 最多写入这么多个 u16。
        let written = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        if written <= 0 {
            return BOOL(1);
        }
        let text = String::from_utf16_lossy(&buffer[..written as usize]);
        if text == search.title {
            search.found = Some(hwnd);
            return BOOL(0); // 找到了，停止枚举
        }
        BOOL(1)
    }

    let mut search = Search {
        title,
        pid: std::process::id(),
        found: None,
    };
    // SAFETY: 回调只读 `search`，且 `search` 在调用期间一直有效。
    let _ = unsafe {
        EnumWindows(
            Some(callback),
            LPARAM((&raw mut search).cast::<core::ffi::c_void>() as isize),
        )
    };
    search.found
}

/// 这个窗口的矩形是否与虚拟桌面有交集（物理像素；我们的进程是 per-monitor-DPI 感知的，
/// 所以 `GetWindowRect` 给的直接就是物理像素，与截图坐标系一致）。
#[cfg(windows)]
fn window_intersects_desktop(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, GetWindowRect, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    let mut rect = RECT::default();
    // SAFETY: 只是查询这个窗口的矩形。
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        // 查不到就当作"还在桌面上"：宁可多等一会儿，也不要把自己的窗口截进图里
        return true;
    }
    // SAFETY: `GetSystemMetrics` 只读系统度量。
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    rect.left < vx + vw && rect.right > vx && rect.top < vy + vh && rect.bottom > vy
}

/// 记下根窗口句柄。读的是 eframe 缓存好的值，每帧调用也无所谓。
    pub fn remember_window(&self, frame: &eframe::Frame) {        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
            if let Ok(handle) = frame.window_handle()
                && let RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                self.hwnd.store(win32.hwnd.get(), Ordering::Relaxed);
            }
        }
        #[cfg(not(windows))]
        let _ = frame;
    }

    /// 根窗口句柄（0 表示还没拿到）。托盘通知需要它当气泡宿主。
    #[must_use]
    pub fn hwnd(&self) -> isize {
        self.hwnd.load(Ordering::Relaxed)
    }

    /// 立刻叫醒 UI 线程（由热键 / 托盘的事件处理器线程调用）。
    pub fn wake(&self, ctx: &egui::Context) {
        *self
            .last_wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::time::Instant::now());

        // 普通路径照旧保留：下面那步即使失败，也只是退回"最多 100 ms"。
        ctx.request_repaint();
        self.nudge();
    }

    /// 只做"叫醒窗口"这一半，不需要 `egui::Context`。
    ///
    /// 单实例转发来的请求（`cuter --pin` 之类）由**另一个线程**收下，它手里没有 `ctx`，
    /// 但必须能把 UI 从待命心跳（最长 4 s）里捅醒——否则外部请求最长要等 4 s 才被处理。
    pub fn nudge(&self) {
        #[cfg(windows)]
        {
            let raw = self.hwnd.load(Ordering::Relaxed);
            if raw == 0 {
                return;
            }
            wake_window(raw);
        }
    }
}

/// 给 eframe 根窗口投一条 `WM_PAINT`，把事件循环从 `WaitUntil` 里提前叫醒。
///
/// 没有参数、没有副作用；失败只影响延迟，不影响正确性，所以只留一条 trace。
#[cfg(windows)]
pub fn wake_window(raw: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_PAINT};
    if raw == 0 {
        return;
    }
    // SAFETY: `raw` 是 eframe 根窗口的 HWND（`is_ready` 之后才非 0）。
    let posted = unsafe {
        PostMessageW(
            Some(HWND(raw as *mut core::ffi::c_void)),
            WM_PAINT,
            WPARAM(0),
            LPARAM(0),
        )
    };
    if let Err(err) = posted {
        log::trace!("投递 WM_PAINT 唤醒失败（退回普通重绘）：{err}");
    }
}

/// 等桌面合成器把"当前这一帧"画完。
///
/// 抓图前用它把合成器推一把：`ShowWindow(SW_HIDE)` 之后，窗口的**样式位**立刻就变了
/// （`IsWindowVisible` 变假），但**屏幕上**那一帧由 DWM 重新合成，要等到下一次提交。
/// 实测（`docs/HANDOFF.md` 第 15 节）就卡在这里：`IsWindowVisible` 已经报假，
/// 立刻 `BitBlt` 拿到的却还是"窗口还在"的那一帧（存下来的图与"我们的窗口在场"的
/// 基准图偏差 26，与干净的基准图偏差应该是 0）。
///
/// `DwmFlush` 会阻塞到 DWM 把这一批绘制提交完；拿不到 DWM 时（返回错误）直接返回。
#[cfg(windows)]
pub fn flush_composition() {
    use windows::Win32::Graphics::Dwm::DwmFlush;
    // SAFETY: 无参数、无前置条件；失败只是没等到，调用方还有时间兜底。
    let _ = unsafe { DwmFlush() };
}

#[cfg(not(windows))]
pub fn flush_composition() {}

// ---------------------------------------------------------------------------
// 热键
// ---------------------------------------------------------------------------

/// 一个可绑定热键的功能。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyCommand {
    /// 框选截图。
    Capture,
    /// 鼠标所在显示器全屏。
    CaptureMonitor,
    /// 整个虚拟桌面。
    CaptureDesktop,
    /// 滚动截屏（M3）。
    ScrollCapture,
    /// 剪贴板图片贴到屏幕。
    PinFromClipboard,
    /// 对上次结果 / 剪贴板做 OCR（M3）。
    Ocr,
    /// 贴图并立刻 OCR（M3）。
    OcrClipboard,
    /// 隐藏 / 显示所有贴图。
    TogglePins,
    /// 切换到下一个贴图分组（当前组整体隐藏、下一组显示）。
    NextPinGroup,
}

impl HotkeyCommand {
    /// 配置里的键名，与 [`HotkeyConfig::entries`] 一一对应。
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::CaptureMonitor => "capture_monitor",
            Self::CaptureDesktop => "capture_desktop",
            Self::ScrollCapture => "scroll_capture",
            Self::PinFromClipboard => "pin_from_clipboard",
            Self::Ocr => "ocr",
            Self::OcrClipboard => "ocr_clipboard",
            Self::TogglePins => "toggle_pins",
            Self::NextPinGroup => "next_pin_group",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Capture => tr!("框选截图"),
            Self::CaptureMonitor => tr!("截取当前显示器"),
            Self::CaptureDesktop => tr!("截取整个桌面"),
            Self::ScrollCapture => tr!("滚动截屏"),
            Self::PinFromClipboard => tr!("贴图（剪贴板）"),
            Self::Ocr => tr!("识别文字"),
            Self::OcrClipboard => tr!("贴图并识别"),
            Self::TogglePins => tr!("隐藏/显示所有贴图"),
            Self::NextPinGroup => tr!("切换到下一个贴图组"),
        }
    }

    pub const ALL: [Self; 9] = [
        Self::Capture,
        Self::CaptureMonitor,
        Self::CaptureDesktop,
        Self::ScrollCapture,
        Self::PinFromClipboard,
        Self::Ocr,
        Self::OcrClipboard,
        Self::TogglePins,
        Self::NextPinGroup,
    ];
}

/// 热键管理器：负责注册/注销，并把系统事件翻译成 [`HotkeyCommand`]。
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    /// 已注册的 `(热键 id, 热键, 功能)`。
    registered: Vec<(u32, HotKey, HotkeyCommand)>,
    /// 注册失败的项（`(键名, 原因)`），显示在设置界面。
    failures: Vec<(String, String)>,
    /// 事件队列，由系统事件处理器（另一个线程）填充。
    rx: Receiver<HotkeyCommand>,
    /// 处理器线程按 `id` 查功能用的查表；与 `registered` 保持同步。
    table: Arc<Mutex<Vec<(u32, HotkeyCommand)>>>,
}

impl Hotkeys {
    /// 初始化热键管理器，并**立刻装上事件处理器**（见模块文档）。
    ///
    /// `ctx` 只用来在事件到达时唤醒 UI；本类型不认识 egui 的其余部分。
    pub fn new(ctx: &egui::Context, wake: UiWake) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| fill!(tr!("热键管理器初始化失败：{}"), e))?;

        let (tx, rx) = channel();
        let table = Arc::new(Mutex::new(Vec::new()));
        let lookup = Arc::clone(&table);
        let ctx = ctx.clone();
        // 注意：底层用的是 `OnceLock`，**一个进程只能设一次**。
        // 本程序只创建一个 `Hotkeys`，所以没问题；将来若要多实例，
        // 得把 handler 改成读取一个全局注册表。
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed
                && let Ok(table) = lookup.lock()
                && let Some((_, command)) = table.iter().find(|(id, _)| *id == event.id)
            {
                let _ = tx.send(*command);
            }
            // 认不认识这个事件都要唤醒一次：用户按下热键后 UI 可能正睡着
            wake.wake(&ctx);
        }));

        Ok(Self {
            manager,
            registered: Vec::new(),
            failures: Vec::new(),
            rx,
            table,
        })
    }

    /// 按配置重新注册全部热键。返回注册失败的原因列表（不致命）。
    pub fn apply(&mut self, config: &HotkeyConfig) -> Vec<String> {
        // `global-hotkey` 只有 `unregister(HotKey)`，所以必须把 HotKey 本身留一份
        for (_, hotkey, _) in self.registered.drain(..) {
            if let Err(err) = self.manager.unregister(hotkey) {
                log::debug!("注销热键失败（可能已经失效）：{err}");
            }
        }
        self.failures.clear();
        let mut warnings = Vec::new();

        for command in HotkeyCommand::ALL {
            let spec = config
                .entries()
                .into_iter()
                .find(|(key, _)| *key == command.key())
                .map(|(_, value)| value.to_owned())
                .unwrap_or_default();
            let spec = spec.trim();
            if spec.is_empty() {
                continue;
            }
            match parse_hotkey(spec) {
                Ok(hotkey) => {
                    let id = hotkey.id();
                    match self.manager.register(hotkey) {
                        Ok(()) => self.registered.push((id, hotkey, command)),
                        Err(err) => {
                            let reason = fill!(tr!("热键「{}」注册失败（可能被其它程序占用）：{}"), spec, err);
                            log::warn!("{reason}");
                            warnings.push(reason.clone());
                            self.failures.push((spec.to_owned(), reason));
                        }
                    }
                }
                Err(reason) => {
                    log::warn!("热键「{spec}」格式错误：{reason}");
                    warnings.push(fill!(tr!("热键「{}」格式错误：{}"), spec, reason));
                    self.failures.push((spec.to_owned(), reason));
                }
            }
        }
        // 重建查表。整体替换而不是就地改：处理器线程随时可能在读它。
        if let Ok(mut table) = self.table.lock() {
            *table = self
                .registered
                .iter()
                .map(|(id, _, command)| (*id, *command))
                .collect();
        }
        // 把"现在到底有哪些热键是活的"写进日志：排查"按了没反应"时这是第一手证据
        // （最典型的坑：键位被别的程序占了，注册失败只在设置界面里提示，用户看不到日志）。
        let live = self
            .registered
            .iter()
            .map(|(_, _, command)| command.label())
            .collect::<Vec<_>>()
            .join(" / ");
        log::info!(
            "已注册热键 {} 个：{}",
            self.registered.len(),
            if live.is_empty() { "（无）" } else { &live }
        );
        warnings
    }

    /// 取出一条已触发的命令（非阻塞）。
    #[must_use]
    pub fn poll(&self) -> Option<HotkeyCommand> {
        self.rx.try_recv().ok()
    }

    #[must_use]
    pub fn failures(&self) -> &[(String, String)] {
        &self.failures
    }
}

/// 解析 `Ctrl+Shift+A` 这类热键字符串。
pub fn parse_hotkey(input: &str) -> Result<HotKey, String> {
    let mut modifiers = Modifiers::empty();
    let mut code: Option<Code> = None;

    for part in input.split('+') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "win" | "super" | "meta" | "cmd" | "command" => modifiers |= Modifiers::SUPER,
            other => {
                if code.is_some() {
                    return Err(fill!(tr!("出现了多个主键（{}）"), other));
                }
                code = Some(parse_code(other).ok_or_else(|| fill!(tr!("无法识别的按键：{}"), other))?);
            }
        }
    }

    let code = code.ok_or_else(|| "缺少主键，例如 Alt+C".to_owned())?;
    if code == Code::KeyC && modifiers.is_empty() {
        // 不禁止，只是提醒：无修饰键的裸字母热键会吃掉全系统的按键
        log::warn!("热键只绑定了裸键 {input}，可能会影响其它程序的输入");
    }
    Ok(HotKey::new(Some(modifiers), code))
}

const LETTERS: [Code; 26] = [
    Code::KeyA,
    Code::KeyB,
    Code::KeyC,
    Code::KeyD,
    Code::KeyE,
    Code::KeyF,
    Code::KeyG,
    Code::KeyH,
    Code::KeyI,
    Code::KeyJ,
    Code::KeyK,
    Code::KeyL,
    Code::KeyM,
    Code::KeyN,
    Code::KeyO,
    Code::KeyP,
    Code::KeyQ,
    Code::KeyR,
    Code::KeyS,
    Code::KeyT,
    Code::KeyU,
    Code::KeyV,
    Code::KeyW,
    Code::KeyX,
    Code::KeyY,
    Code::KeyZ,
];

const DIGITS: [Code; 10] = [
    Code::Digit0,
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
];

const FUNCTIONS: [Code; 24] = [
    Code::F1,
    Code::F2,
    Code::F3,
    Code::F4,
    Code::F5,
    Code::F6,
    Code::F7,
    Code::F8,
    Code::F9,
    Code::F10,
    Code::F11,
    Code::F12,
    Code::F13,
    Code::F14,
    Code::F15,
    Code::F16,
    Code::F17,
    Code::F18,
    Code::F19,
    Code::F20,
    Code::F21,
    Code::F22,
    Code::F23,
    Code::F24,
];

fn parse_code(token: &str) -> Option<Code> {
    let upper = token.to_ascii_uppercase();

    if upper.len() == 1 {
        let byte = upper.as_bytes()[0];
        if byte.is_ascii_uppercase() {
            return Some(LETTERS[(byte - b'A') as usize]);
        }
        if byte.is_ascii_digit() {
            return Some(DIGITS[(byte - b'0') as usize]);
        }
    }

    if let Some(rest) = upper.strip_prefix('F')
        && let Ok(number) = rest.parse::<usize>()
        && (1..=24).contains(&number)
    {
        return Some(FUNCTIONS[number - 1]);
    }

    Some(match upper.as_str() {
        "PRINTSCREEN" | "PRTSC" | "SNAPSHOT" => Code::PrintScreen,
        "SPACE" => Code::Space,
        "ENTER" | "RETURN" => Code::Enter,
        "TAB" => Code::Tab,
        "ESC" | "ESCAPE" => Code::Escape,
        "BACKSPACE" => Code::Backspace,
        "DELETE" | "DEL" => Code::Delete,
        "INSERT" | "INS" => Code::Insert,
        "HOME" => Code::Home,
        "END" => Code::End,
        "PAGEUP" | "PGUP" => Code::PageUp,
        "PAGEDOWN" | "PGDN" => Code::PageDown,
        "UP" | "ARROWUP" => Code::ArrowUp,
        "DOWN" | "ARROWDOWN" => Code::ArrowDown,
        "LEFT" | "ARROWLEFT" => Code::ArrowLeft,
        "RIGHT" | "ARROWRIGHT" => Code::ArrowRight,
        "`" | "BACKQUOTE" => Code::Backquote,
        "-" | "MINUS" => Code::Minus,
        "=" | "EQUAL" => Code::Equal,
        "," | "COMMA" => Code::Comma,
        "." | "PERIOD" => Code::Period,
        "/" | "SLASH" => Code::Slash,
        ";" | "SEMICOLON" => Code::Semicolon,
        "'" | "QUOTE" => Code::Quote,
        "[" | "BRACKETLEFT" => Code::BracketLeft,
        "]" | "BRACKETRIGHT" => Code::BracketRight,
        "\\" | "BACKSLASH" => Code::Backslash,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// 开机自启
// ---------------------------------------------------------------------------
//
// 实现方式是 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的一个 REG_SZ。
//
// 为什么用 HKCU 而不是「任务计划程序」或「启动文件夹」：
//
// * **不需要管理员权限**（HKLM 需要，而截图工具没有理由要提权）；
// * 卸载/关闭时能干净删掉（启动文件夹是文件，用户还会以为那是快捷方式）；
// * 任务计划程序那套要写 XML，能做的事情（延迟启动、条件）我们暂时用不到。

/// 注册表里的值名。
const AUTOSTART_VALUE: &str = "Cuter";

/// 自启命令行：**必须带 `--tray`**。
///
/// 否则用户一旦把「启动即驻留托盘」关掉，开机就会蹦出一个设置窗口。
/// 路径加引号：装在 `C:\Program Files\...` 这种带空格的目录下时，不加引号会被
/// Windows 解析成两个参数。
#[must_use]
pub fn autostart_command(exe: &std::path::Path) -> String {
    format!("\"{}\" --tray", exe.display())
}

/// 注册表里那条命令是不是指向这个 exe。
///
/// 用来判断"这条自启项是我们的吗"：**便携版默认不开自启，它启动时绝不能把已安装
/// 版本的项顺手删掉**（两者用的是同一个值名）。
#[must_use]
pub fn entry_targets(entry: &str, exe: &std::path::Path) -> bool {
    let entry = entry.trim();
    let head = if let Some(rest) = entry.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        // 没加引号：按第一个空格断开。路径本身带空格时这种写法本来就是坏的，
        // 但在 Windows 上确实常见（比如老版本工具写的），能认出来是好事。
        entry.split(' ').next().unwrap_or("")
    };
    !head.is_empty() && head.eq_ignore_ascii_case(&exe.to_string_lossy())
}

/// 读回自启项；不存在时返回 `None`。
pub fn autostart_entry() -> Option<String> {
    platform::autostart_read(AUTOSTART_VALUE)
}

/// 写入（`enabled = true`）或删除（`false`）自启项。
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| fill!(tr!("取不到自身路径：{}"), e))?;
    platform::autostart_write(AUTOSTART_VALUE, enabled.then(|| autostart_command(&exe)))
}

/// 启动时的对账：让注册表与配置一致，但**不越权**。
///
/// * 配置说开 → 把值写成当前 exe（用户把文件夹搬走之后也能自愈）；
/// * 配置说关 → **只删指向本 exe 的那一条**，别人的自启项一根汗毛都不动。
pub fn reconcile_autostart(enabled: bool) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let current = autostart_entry();
    if enabled {
        let want = autostart_command(&exe);
        if current.as_deref() != Some(want.as_str()) {
            match set_autostart(true) {
                Ok(()) => log::info!("已写入开机自启：{want}"),
                Err(err) => log::warn!("写入开机自启失败：{err}"),
            }
        }
    } else if current.as_deref().is_some_and(|entry| entry_targets(entry, &exe))
        && let Err(err) = set_autostart(false)
    {
        log::warn!("删除开机自启失败：{err}");
    }
}

#[cfg(windows)]
mod platform {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW,
    };
    use windows::core::{PCWSTR, w};

    /// 自启项所在的位置。
    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

    /// 读回一个 REG_SZ 值。
    pub(super) fn autostart_read(name: &str) -> Option<String> {
        let mut key = HKEY::default();
        // SAFETY: 只读查询；成功后句柄在下面关掉。
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_QUERY_VALUE, &mut key) };
        if opened.is_err() {
            return None;
        }
        let wide = to_wide(name);
        let mut kind = REG_SZ;
        let mut buffer = vec![0u8; 1024];
        let mut size = u32::try_from(buffer.len()).expect("缓冲区长度");
        // SAFETY: `buffer`/`size` 成对传入，驱动最多写 `size` 字节。
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(wide.as_ptr()),
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        // SAFETY: `key` 是上面成功打开的句柄。
        unsafe {
            let _ = RegCloseKey(key);
        }
        if status.is_err() {
            return None;
        }
        let used = usize::try_from(size).unwrap_or(0).min(buffer.len());
        let units: Vec<u16> = buffer[..used]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    /// 写入（`Some`）或删除（`None`）一个 REG_SZ 值。
    pub(super) fn autostart_write(name: &str, value: Option<String>) -> Result<(), String> {
        let mut key = HKEY::default();
        // SAFETY: 创建/打开 HKCU 下的子键；返回值会检查。
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_QUERY_VALUE,
                None,
                &mut key,
                None,
            )
        };
        check(status, "打开 Run 键")?;

        let wide_name = to_wide(name);
        let result = match value {
            Some(text) => {
                let wide = to_wide(&text);
                let bytes: Vec<u8> = wide
                    .iter()
                    .flat_map(|unit| unit.to_le_bytes())
                    .collect();
                // SAFETY: `bytes` 是 UTF-16（含结尾 NUL）的字节表示，长度与内容一致。
                let status = unsafe {
                    RegSetValueExW(
                        key,
                        PCWSTR(wide_name.as_ptr()),
                        None,
                        REG_SZ,
                        Some(&bytes),
                    )
                };
                check(status, "写入自启项")
            }
            None => {
                // SAFETY: 删除一个可能不存在的值；"找不到"是正常情况，不算失败。
                let status = unsafe { RegDeleteValueW(key, PCWSTR(wide_name.as_ptr())) };
                if status.is_err() && status != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                    check(status, "删除自启项")
                } else {
                    Ok(())
                }
            }
        };
        // SAFETY: `key` 来自上面成功的 `RegCreateKeyExW`。
        unsafe {
            let _ = RegCloseKey(key);
        }
        result
    }

    fn check(status: WIN32_ERROR, what: &str) -> Result<(), String> {
        if status.is_ok() {
            Ok(())
        } else {
            Err(fill!(tr!("{}失败：{}"), status.to_hresult().message(), what))
        }
    }

    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    pub(super) fn autostart_read(_name: &str) -> Option<String> {
        None
    }

    pub(super) fn autostart_write(_name: &str, _value: Option<String>) -> Result<(), String> {
        Err("当前平台还没有实现开机自启".to_owned())
    }
}

// ---------------------------------------------------------------------------
// 剪贴板
// ---------------------------------------------------------------------------

/// 把一张图放进系统剪贴板。
pub fn copy_image(frame: &Frame) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| fill!(tr!("打开剪贴板失败：{}"), e))?;
    clipboard
        .set_image(arboard::ImageData {
            width: frame.size().w as usize,
            height: frame.size().h as usize,
            bytes: Cow::Borrowed(frame.data()),
        })
        .map_err(|e| fill!(tr!("写入剪贴板失败：{}"), e))?;
    Ok(())
}

/// 把文本放进剪贴板（OCR 结果复制用）。
pub fn copy_text(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| fill!(tr!("打开剪贴板失败：{}"), e))?;
    clipboard
        .set_text(text)
        .map_err(|e| fill!(tr!("写入剪贴板失败：{}"), e))
}

/// 从剪贴板取一张图。
pub fn paste_image() -> Result<Frame, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| fill!(tr!("打开剪贴板失败：{}"), e))?;
    let image = clipboard
        .get_image()
        .map_err(|e| fill!(tr!("剪贴板里没有图片：{}"), e))?;
    let size = cuter_core::geom::Size::new(image.width as u32, image.height as u32);
    Frame::from_rgba(size, image.bytes.into_owned())
        .map_err(|e| fill!(tr!("剪贴板图片尺寸异常：{}"), e))
}

// ---------------------------------------------------------------------------
// 贴图拖出（OLE 拖放）
// ---------------------------------------------------------------------------

/// 贴图拖出时用的临时目录（`%TEMP%\Cuter\drag`）。
///
/// 为什么落成文件而不是走 `CF_DIB` 内存格式：**所有目标程序都认文件**
/// （资源管理器、微信 / QQ、浏览器、VSCode、Discord…），而 `CF_DIB`
/// 需要自己实现 `IStream`，兼容面反而更窄。放 `%TEMP%` 是因为它是"可丢弃"的，
/// 而且拖放用的是 `DROPEFFECT_COPY`（目标程序会复制，不会把文件搬走）。
#[must_use]
pub fn drag_temp_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("Cuter").join("drag")
}

/// 清掉一天前的拖出临时文件（不然 `%TEMP%` 会一直攒）。
pub fn prune_drag_temp_dir() {
    let dir = drag_temp_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            log::debug!("清掉旧的拖出临时文件：{:?}", entry.path());
        }
    }
}

/// 把若干文件"拖"到别的程序里（OLE 拖放），**阻塞到用户放下或取消**。
///
/// 返回一句可以显示给用户的结果。实现要点：
///
/// * 数据对象用 **shell 自己造的**（`SHCreateDataObject` + 文件 PIDL）：
///   手写 `IDataObject` + `IEnumFORMATETC` + `HGLOBAL` 所有权又长又容易错，
///   而 shell 的实现同时提供 `CF_HDROP` 与 shell 自己的格式，兼容性最好。
/// * 我们自己只需要实现**拖放源**（`IDropSource`，两个方法）。
/// * `DoDragDrop` 会跑一个模态消息循环：期间 winit 的 WndProc 只是把消息排队，
///   不会重入我们的 `update()`（winit 的循环此刻正阻塞在这一层里），
///   所以"在 egui 的帧里起拖放"是安全的。
pub fn drag_files(paths: &[std::path::PathBuf]) -> Result<String, String> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{S_FALSE, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS};
        use windows::Win32::System::Com::IDataObject;
        use windows::Win32::System::Ole::{
            DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, DROPEFFECT, DROPEFFECT_COPY,
        };
        use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
        use windows::Win32::UI::Shell::Common::ITEMIDLIST;
        use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHCreateDataObject};
        use windows::core::{implement, PCWSTR, HRESULT};

        if paths.is_empty() {
            return Err("没有要拖出的文件".to_owned());
        }

        // 拖放源：Esc 取消；**按下时用的那个键松开**就算"放下"。
        // 我们用 Ctrl+左键拖动，所以这里看左键。
        #[implement(IDropSource)]
        struct PinDragSource;

        impl IDropSource_Impl for PinDragSource_Impl {
            fn QueryContinueDrag(
                &self,
                fescapepressed: windows::core::BOOL,
                grfkeystate: MODIFIERKEYS_FLAGS,
            ) -> HRESULT {
                // 只要**还有任意一个键按着**就继续拖：我们有两种起拖手势
                // （Ctrl+左键、中键），只看左键会把中键那次判成"已松手"，
                // 于是拖放在起点就立刻结束（实测踩过）。
                use windows::Win32::System::SystemServices::{MK_LBUTTON, MK_MBUTTON, MK_RBUTTON};
                if fescapepressed.as_bool() {
                    return DRAGDROP_S_CANCEL;
                }
                let held = MK_LBUTTON.0 | MK_MBUTTON.0 | MK_RBUTTON.0;
                if grfkeystate.0 & held == 0 {
                    return DRAGDROP_S_DROP;
                }
                HRESULT(0) // S_OK：继续拖
            }

            fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
                DRAGDROP_S_USEDEFAULTCURSORS
            }
        }

        // 每个线程初始化一次 OLE（重复调用返回 S_FALSE，无害）。
        // **不调 `OleUninitialize`**：万一这个线程是别的东西先初始化的，
        // 我们反初始化会把它拆掉。
        std::thread_local! {
            static OLE_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        let already = OLE_READY.with(|flag| flag.replace(true));
        if !already {
            // SAFETY: 传 null 表示按默认初始化；S_FALSE 表示已经初始化过，同样可用。
            match unsafe { OleInitialize(None) } {
                Ok(()) => {}
                Err(err) if err.code() == S_FALSE => {}
                Err(err) => return Err(fill!(tr!("初始化 OLE 失败：{}"), err)),
            }
        }

        // UTF-16 + 结尾 0
        let wide: Vec<u16> = paths
            .iter()
            .flat_map(|path| {
                let mut units: Vec<u16> =
                    path.as_os_str().to_string_lossy().encode_utf16().collect();
                units.push(0);
                units
            })
            .collect();

        // SAFETY: 下面全部是 shell / OLE 的标准调用序列；PIDL 由 `ILCreateFromPathW` 分配、
        // 由 `ILFree` 释放，数据对象在 `DoDragDrop` 返回前一直活着。
        unsafe {
            let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
            for unit in wide.split(|c| *c == 0).filter(|s| !s.is_empty()) {
                let mut text = unit.to_vec();
                text.push(0);
                let pidl = ILCreateFromPathW(PCWSTR(text.as_ptr()));
                if pidl.is_null() {
                    for pidl in &pidls {
                        ILFree(Some(*pidl));
                    }
                    return Err("找不到要拖出的文件（PIDL 创建失败）".to_owned());
                }
                pidls.push(pidl);
            }

            let data: Result<IDataObject, _> =
                SHCreateDataObject(None, Some(&pidls), None);
            let data = match data {
                Ok(data) => data,
                Err(err) => {
                    for pidl in &pidls {
                        ILFree(Some(*pidl));
                    }
                    return Err(fill!(tr!("构造拖放数据失败：{}"), err));
                }
            };

            let source: IDropSource = PinDragSource.into();
            let mut effect = DROPEFFECT(0);
            let result = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
            for pidl in &pidls {
                ILFree(Some(*pidl));
            }
            let effect = effect.0;
            log::info!(
                "拖放结束：effect={:#x} hr={:?}（COPY={:#x}）",
                effect,
                result,
                DROPEFFECT_COPY.0
            );
            // `DoDragDrop` 在这个版本上返回裸 `HRESULT`（不是 `Result`）：
            // `DRAGDROP_S_DROP` / `DRAGDROP_S_CANCEL` 都是**成功**级别，只有真正的失败才报错。
            if result.is_err() {
                return Err(fill!(tr!("拖放失败：{}"), result));
            }
            Ok(if effect & DROPEFFECT_COPY.0 != 0 {
                fill!(tr!("已拖出 {} 个文件"), paths.len())
            } else {
                "拖放已取消（目标程序没有接收）".to_owned()
            })
        }
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        Err("当前平台尚未实现拖出".to_owned())
    }
}

// ---------------------------------------------------------------------------
// 托盘
// ---------------------------------------------------------------------------

/// 托盘菜单项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Capture,
    CaptureMonitor,
    PinFromClipboard,
    /// 右键单击托盘图标：打开自绘的托盘菜单面板（`cuter-ui::tray_panel`）。
    ///
    /// 以前这一项由 Win32 原生菜单承担，现在是宿主自己画的窗口——位置由
    /// `App::open_tray_menu` 按鼠标当前位置算（贴屏幕边时会被夹进工作区）。
    OpenMenu,
    /// 隐藏 / 显示屏幕上所有贴图。
    TogglePins,
    /// 取消所有贴图的鼠标穿透。**逃生项**：穿透开着时贴图窗口收不到鼠标事件，
    /// 菜单里那颗开关根本点不到，只有窗口外的入口能救回来。
    ClearPinClickThrough,
    /// 切换到下一个贴图分组。
    NextPinGroup,
    /// 打开截图历史面板。
    OpenHistory,
    OpenSettings,
    OpenSaveDir,
    Quit,
}

/// 托盘图标 + 菜单。
pub struct Tray {
    /// 托盘图标句柄。**保留**它才能换图标（主题皮肤切换时要换），
    /// 所以不能像早先那样叫 `_icon` 丢掉。
    icon: TrayIcon,
    /// 事件队列，由菜单/图标事件处理器（另一个线程）填充。
    rx: Receiver<TrayCommand>,
}

/// 托盘悬浮提示：把最常用的三个键位写在里面（同类工具的托盘提示也承担这个作用）。
fn tray_tooltip(config: &cuter_core::config::Config) -> String {
    let hotkeys = &config.hotkeys;
    let mut parts = Vec::new();
    if !hotkeys.capture.trim().is_empty() {
        parts.push(fill!(tr!("截图 {}"), hotkeys.capture.trim()));
    }
    if !hotkeys.scroll_capture.trim().is_empty() {
        parts.push(fill!(tr!("滚动截屏 {}"), hotkeys.scroll_capture.trim()));
    }
    if !hotkeys.pin_from_clipboard.trim().is_empty() {
        parts.push(fill!(tr!("贴图 {}"), hotkeys.pin_from_clipboard.trim()));
    }
    if parts.is_empty() {
        "Cuter —— 截图 / 标注 / 贴图".to_owned()
    } else {
        format!("Cuter —— {}", parts.join(" · "))
    }
}

impl Tray {
    /// 创建托盘图标并**立刻装上事件处理器**（见模块文档）。
    ///
    /// `config` 用来在菜单项后面显示各自的快捷键、以及设置托盘悬浮提示。
    pub fn new(
        ctx: &egui::Context,
        wake: UiWake,
        config: &cuter_core::config::Config,
    ) -> Result<Self, String> {
        let icon = tray_icon::Icon::from_rgba(crate::icon::tray_icon_rgba(), 32, 32)
            .map_err(|e| fill!(tr!("托盘图标数据非法：{}"), e))?;

        // 托盘右键菜单不再用 Win32 原生菜单（`muda`）：它和界面其余部分完全是两套样子，
        // 主题切换时只有它不变（设计系统 §11 把"托盘右键菜单"列为还没覆盖的界面）。
        // 现在右键打开的是 `cuter-ui` 里自绘的 `tray_panel`——无边框、总在最前的独立视口，
        // 和贴图右键菜单、截图历史面板共用同一套设计令牌。
        // 左键单击保留"直接框选截图"的老习惯（同类工具的左键也多是直接框选）。
        let tray = TrayIconBuilder::new()
            .with_tooltip(tray_tooltip(config))
            .with_icon(icon)
            .build()
            .map_err(|e| fill!(tr!("创建托盘图标失败：{}"), e))?;

        let (tx, rx) = channel();

        let click_ctx = ctx.clone();
        let click_wake = wake;
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let command = match event {
                // 左键单击：很多用户的第一反应就是点托盘图标
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => Some(TrayCommand::Capture),
                // 右键单击：打开自绘菜单（位置由宿主按当时的鼠标位置算）
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    ..
                } => Some(TrayCommand::OpenMenu),
                _ => None,
            };
            if let Some(command) = command {
                let _ = tx.send(command);
            }
            click_wake.wake(&click_ctx);
        }));

        Ok(Self { icon: tray, rx })
    }

    /// 取出一条托盘命令（菜单点击或图标单击，非阻塞）。
    #[must_use]
    pub fn poll(&self) -> Option<TrayCommand> {
        self.rx.try_recv().ok()
    }

    /// 换一套托盘图标（界面主题皮肤变了时调用）。
    ///
    /// 失败只记日志：图标没换掉不影响任何功能，不值得为此打断用户。
    pub fn set_skin(&self, skin: crate::icon::IconSkin) {
        let size = crate::icon::TRAY_SIZE;
        match tray_icon::Icon::from_rgba(crate::icon::tray_icon_rgba_for(skin), size, size) {
            Ok(icon) => match self.icon.set_icon(Some(icon)) {
                Ok(()) => log::info!("托盘图标已换成皮肤 {}", skin.code()),
                Err(err) => log::warn!("换托盘图标失败：{err}"),
            },
            Err(err) => log::warn!("生成托盘图标失败：{err}"),
        }
    }
}

/// 系统界面语言：中文系统给中文，其它给英文。
///
/// 放在这里而不是 `cuter-ui`：那个 crate 的硬约定是**不碰任何平台 API**，
/// 所以"跟随系统"由宿主解析完再交给它（`Language::parse(value, fallback)`）。
#[must_use]
pub fn system_language() -> cuter_ui::i18n::Language {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    // windows-rs 的包装直接收 `&mut [u16]`（LOCALE_NAME_MAX_LENGTH = 85）
    let mut buffer = [0u16; 85];
    // SAFETY: 传入的是本进程的栈缓冲；返回值是写入的字符数（含结尾的 0）。
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if len > 0 {
        let name = String::from_utf16_lossy(&buffer[..(len as usize - 1).min(buffer.len())]);
        if name.to_ascii_lowercase().starts_with("zh") {
            return cuter_ui::i18n::Language::ZhCn;
        }
        return cuter_ui::i18n::Language::EnUs;
    }
    cuter_ui::i18n::Language::ZhCn
}

/// 从**进程创建**到现在的毫秒数。
///
/// 用来给启动画一条时间线：`main()` 执行时 CRT 与 exe 装载早就完成了，所以只从
/// `main()` 里打点量不出"用户双击到看见画面"的时间。这里问系统要进程创建时间，
/// 两个时间戳都走同一只时钟（`GetSystemTimeAsFileTime`），避免时区/纪元换算。
#[must_use]
pub fn since_process_start_ms() -> u128 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn ticks(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }

    let mut created = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: 四个 FILETIME 都是本进程栈上的可写位置；句柄是伪句柄，不需要释放。
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return 0;
    }
    // SAFETY: 无参数、无所有权的只读查询。
    let now = unsafe { GetSystemTimeAsFileTime() };
    ticks(now).saturating_sub(ticks(created)) as u128 / 10_000
}

/// 当前前台窗口所属进程的可执行文件名（不含扩展名）。
///
/// 文件名模板的 `{app}` 用它。必须在**按下热键那一刻**调用：等真正保存时前台已经是
/// 我们自己的遮罩/面板了，那时只会得到 "cuter"。取不到就返回空串（模板里会变成空）。
#[must_use]
pub fn foreground_app_name() -> String {
    use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    // SAFETY: 全部是只读查询；失败路径统一返回空串。
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return String::new();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buffer = [0u16; MAX_PATH as usize];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buffer[..len as usize]);
        // 取文件名再去掉扩展名：`C:\...\Code.exe` → `Code`
        std::path::Path::new(&full)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}

/// 托盘闲置时把工作集还给系统，返回（收紧前 MB，收紧后 MB）。
///
/// 只改"任务管理器里看到的数字"：私有提交量不变，被换出去的页在下次访问时按需换回，
/// 所以不能对用户说"省了这么多内存"。托盘待命是这个程序的常态（绝大多数时间没有窗口），
/// 让它挂着一百多兆工作集没有意义；收紧一次之后不再反复触发，避免页错误来回折腾。
///
/// 返回 `None` 表示系统调用失败（不致命，保持原状）。
#[must_use]
pub fn trim_working_set() -> Option<(u64, u64)> {
    use windows::Win32::System::ProcessStatus::{
        EmptyWorkingSet, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    #[allow(clippy::cast_possible_truncation)]
    fn working_set_mb() -> Option<u64> {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        // SAFETY: 结构体由本函数持有，大小如实上报；句柄是当前进程的伪句柄。
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &raw mut counters,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
            .ok()?;
        }
        Some(counters.WorkingSetSize as u64 / (1024 * 1024))
    }

    let before = working_set_mb()?;
    // SAFETY: 当前进程的伪句柄，参数固定为 -1（全部页）。
    let ok = unsafe { EmptyWorkingSet(GetCurrentProcess()) }.is_ok();
    if !ok {
        return None;
    }
    let after = working_set_mb()?;
    Some((before, after))
}

/// 往选区中央发一次滚轮事件（自动滚动模式用）。
///
/// 为什么是 `SendInput` 而不是 `PostMessage(WM_MOUSEWHEEL)`：后者是"发给某个窗口的消息"，
/// 现代程序（浏览器、Electron、WPF 的虚拟化列表）大多不理会合成消息，只认真正进过
/// 输入队列的事件。而 `SendInput` 的事件会走一遍系统输入栈，落到**光标底下那个窗口**上
/// ——所以必须先把光标移到选区中央（移完再放回原处，用户只会看到它跳一下）。
///
/// 返回是否成功投递。目标程序忽略滚轮时这里仍然返回真：那属于"没人消费"，
/// 由 `AutoScrollTracker` 用"连续几帧没变化"判定并停下。
pub fn auto_scroll_step(region: cuter_core::geom::Rect, horizontal: bool) -> bool {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GA_ROOT, GetAncestor, GetCursorPos, SetCursorPos, SetForegroundWindow, WindowFromPoint,
    };

    const WHEEL_DELTA: i32 = 120;
    /// 一次滚三格：太小会让人等，太大容易跳过内容（拼接会跟丢）。
    const NOTCHES: i32 = 3;

    let mut saved = POINT::default();
    // SAFETY: 只是读一次光标位置；失败就当"没有原位置可恢复"。
    let had_saved = unsafe { GetCursorPos(&mut saved) }.is_ok();

    let centre = cuter_core::geom::Point {
        x: region.x + region.w as i32 / 2,
        y: region.y + region.h as i32 / 2,
    };
    // SAFETY: 移动光标到选区中央。坐标来自用户自己框出的选区，一定在虚拟桌面内。
    unsafe {
        let _ = SetCursorPos(centre.x, centre.y);
    }

    // 滚轮落到光标底下那个窗口上。选区中心如果被别的窗口盖着（切走的应用、刚关掉的设置
    // 窗口、探针之间残留的窗口），这次滚动就会去滚别人，用户看到的是"自动滚动没反应"。
    // 先把这一点的顶层窗口提到前台——人手滚动之前做的也是这件事。
    // SAFETY: 只做查询与置前，句柄用完即弃；任何一步失败都保持"直接发滚轮"的原行为。
    unsafe {
        let probe = POINT {
            x: centre.x,
            y: centre.y,
        };
        let under = WindowFromPoint(probe);
        if !under.is_invalid() {
            let root = GetAncestor(under, GA_ROOT);
            if !root.is_invalid() {
                let _ = SetForegroundWindow(root);
            }
        }
    }

    // 两个方向都用**负**增量，这不是笔误：`MOUSEEVENTF_WHEEL` 正数向上滚（我们要向下），
    // 而 `MOUSEEVENTF_HWHEEL` 的符号约定是反的（正数向左滚，我们要向右）。
    let data = -WHEEL_DELTA * NOTCHES;
    let flags = if horizontal {
        MOUSEEVENTF_HWHEEL
    } else {
        MOUSEEVENTF_WHEEL
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: 一个合法的 INPUT 结构体，长度按 size_of 给出。
    let inserted = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };

    if had_saved {
        // SAFETY: 把光标放回原处，别把用户的鼠标留在选区里。
        unsafe {
            let _ = SetCursorPos(saved.x, saved.y);
        }
    }
    inserted == 1
}

// ---------------------------------------------------------------------------
// 托盘气泡通知
// ---------------------------------------------------------------------------

/// 托盘气泡通知器。
///
/// **为什么自己写**：`tray-icon` 只提供图标/菜单/提示文字，没有气泡接口，
/// 而且它的 `hwnd` 与 `uID` 都是私有的——没法往它那个图标上挂 `NIF_INFO`。
/// 所以这里注册一个**隐藏的辅助图标**（`NIS_HIDDEN` + 我们自己的根窗口句柄 +
/// 固定 `uID`）专门当气泡宿主：外观由系统画（Win10+ 会转成系统通知），
/// 现有托盘图标的实现一行都不用动。
///
/// 生命周期：第一次通知时 `NIM_ADD`，之后复用（每次 `NIM_MODIFY` + `NIF_INFO`），
/// 析构时 `NIM_DELETE`。任何一步失败都只记日志——通知失败绝不能影响截图。
pub struct TrayNotifier {
    hwnd: isize,
    enabled: bool,
    registered: bool,
}

/// 辅助图标的固定 ID（和 `tray-icon` 那个图标互不干扰）。
const NOTIFY_ICON_ID: u32 = 0xC0_7E;

impl TrayNotifier {
    /// `hwnd` 是自绘 UI 的根窗口句柄；`enabled` 来自 `general.show_tray_notifications`。
    #[must_use]
    pub fn new(hwnd: isize, enabled: bool) -> Self {
        Self {
            hwnd,
            enabled,
            registered: false,
        }
    }

    /// 配置开关变了（设置界面里勾选）。
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 弹一条通知。返回是否真的交给了系统（探针据此判断开关是否生效）。
    pub fn notify(&mut self, title: &str, body: &str) -> bool {
        if !self.enabled || self.hwnd == 0 {
            return false;
        }
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::Shell::{
                NIF_ICON, NIF_INFO, NIF_STATE, NIF_TIP, NIIF_INFO, NIIF_NOSOUND,
                NIM_ADD,
                NIM_MODIFY, NIS_HIDDEN, NOTIFYICONDATAW, Shell_NotifyIconW,
            };
            use windows::Win32::UI::WindowsAndMessaging::{IDI_APPLICATION, LoadIconW};

            let hwnd = HWND(self.hwnd as *mut core::ffi::c_void);
            // SAFETY: 只加载一个系统预定义图标，不涉及所有权转移。
            let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default();

            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: NOTIFY_ICON_ID,
                ..Default::default()
            };
            if !self.registered {
                // 图标本身藏起来（`NIS_HIDDEN`）：它只是气泡的宿主，不该在通知区域多出一个图标。
                data.uFlags = NIF_ICON | NIF_TIP | NIF_STATE;
                data.hIcon = icon;
                data.dwState = NIS_HIDDEN;
                data.dwStateMask = NIS_HIDDEN;
                let tip: Vec<u16> = "Cuter".encode_utf16().collect();
                data.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
                // SAFETY: `data` 是本栈上的合法结构体，`cbSize` 已按结构体大小填好。
                let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
                if !added {
                    log::warn!("托盘通知：注册辅助图标失败，本次不弹气泡");
                    return false;
                }
                self.registered = true;
            }

            let title: Vec<u16> = title.encode_utf16().collect();
            let body: Vec<u16> = body.encode_utf16().collect();
            data.uFlags = NIF_INFO;
            data.szInfo[..body.len().min(255)].copy_from_slice(&body[..body.len().min(255)]);
            data.szInfoTitle[..title.len().min(63)]
                .copy_from_slice(&title[..title.len().min(63)]);
            // 不额外发声：截图/保存的提示音由 `capture.play_sound` 单独控制，
            // 通知再响一次就成了噪音。
            data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
            // SAFETY: 同上，`data` 合法且 `hWnd`/`uID` 与注册时一致。
            let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool();
            if !ok {
                log::warn!("托盘通知：气泡投放失败（{title:?}）");
            }
            ok
        }
        #[cfg(not(windows))]
        {
            let _ = (title, body);
            false
        }
    }
}

impl Drop for TrayNotifier {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::Shell::{NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: HWND(self.hwnd as *mut core::ffi::c_void),
                uID: NOTIFY_ICON_ID,
                ..Default::default()
            };
            data.uFlags = windows::Win32::UI::Shell::NIF_ICON;
            // SAFETY: 与注册时同样的 hWnd/uID，删除自己注册的辅助图标。
            // 返回值是"删掉了吗"，析构里没什么可做的，显式丢弃。
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
        }
        self.registered = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alt_c() {
        let hotkey = parse_hotkey("Alt+C").expect("Alt+C 应能解析");
        assert_eq!(hotkey.mods, Modifiers::ALT);
        assert_eq!(hotkey.key, Code::KeyC);
    }

    #[test]
    fn parse_is_case_and_space_insensitive() {
        let a = parse_hotkey("ctrl + shift + a").unwrap();
        assert_eq!(a.mods, Modifiers::CONTROL | Modifiers::SHIFT);
        assert_eq!(a.key, Code::KeyA);
    }

    #[test]
    fn parse_super_aliases() {
        for alias in ["Win+P", "Super+P", "Meta+P", "Cmd+P"] {
            let hotkey = parse_hotkey(alias).unwrap_or_else(|e| panic!("{alias} 解析失败：{e}"));
            assert_eq!(hotkey.mods, Modifiers::SUPER, "{alias}");
        }
    }

    #[test]
    fn parse_function_and_digit_keys() {
        assert_eq!(parse_hotkey("F12").unwrap().key, Code::F12);
        assert_eq!(parse_hotkey("Alt+1").unwrap().key, Code::Digit1);
        assert_eq!(parse_hotkey("Ctrl+PrintScreen").unwrap().key, Code::PrintScreen);
        assert_eq!(parse_hotkey("Alt+Left").unwrap().key, Code::ArrowLeft);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_hotkey("").is_err());
        assert!(parse_hotkey("Alt").is_err(), "只有修饰键应当报错");
        assert!(parse_hotkey("Alt+NotAKey").is_err());
        assert!(parse_hotkey("Alt+C+D").is_err(), "两个主键应当报错");
        assert!(parse_hotkey("F25").is_err(), "超出 F1-F24");
    }

    #[test]
    fn every_hotkey_command_has_a_config_key() {
        let config = HotkeyConfig::default();
        let keys: Vec<&str> = config.entries().into_iter().map(|(k, _)| k).collect();
        for command in HotkeyCommand::ALL {
            assert!(
                keys.contains(&command.key()),
                "功能 {:?} 的配置键 {} 不存在于 HotkeyConfig::entries()",
                command,
                command.key()
            );
        }
        assert_eq!(keys.len(), HotkeyCommand::ALL.len());
    }

    #[test]
    fn tray_icon_rgba_has_expected_length() {
        let rgba = crate::icon::tray_icon_rgba();
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert!(
            rgba.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
            "托盘图标不能是全透明的"
        );
    }

    /// 自启命令必须是"带引号的路径 + `--tray`"：装在带空格的目录下也不能被拆开。
    #[test]
    fn autostart_command_quotes_the_path_and_forces_tray() {
        let exe = std::path::Path::new(r"C:\Program Files\Cuter\cuter.exe");
        assert_eq!(
            autostart_command(exe),
            r#""C:\Program Files\Cuter\cuter.exe" --tray"#
        );
    }

    /// 判断"这条自启项是不是我们的"：便携版就是靠它才敢不去动别人写的项。
    #[test]
    fn entry_targets_only_matches_our_own_exe() {
        let exe = std::path::Path::new(r"C:\Apps\Cuter\cuter.exe");

        assert!(entry_targets(
            r#""C:\Apps\Cuter\cuter.exe" --tray"#,
            exe
        ));
        assert!(entry_targets(r"C:\Apps\Cuter\cuter.exe --tray", exe));
        assert!(
            entry_targets(r#""c:\apps\cuter\CUTER.EXE""#, exe),
            "Windows 路径不区分大小写"
        );
        assert!(!entry_targets(r#""D:\Other\cuter.exe" --tray"#, exe));
        assert!(!entry_targets("", exe));
        assert!(!entry_targets(r#""C:\Apps\Cuter\cuter-lite.exe""#, exe));
    }
}
