//! 控件级吸附：问 UI Automation 要"这个点下面是哪个控件"。
//!
//! 窗口级吸附（`win_windows`）只能吸到整窗边界；用户想截"那个按钮""那个输入框"时，
//! 还得手动去描。这里用系统自带的 UI Automation（`IUIAutomation::ElementFromPoint`）
//! 拿到光标下**最深**那个控件的屏幕矩形。
//!
//! 三条实现上的讲究：
//!
//! 1. **`ElementFromPoint` 不便宜**（跨进程 COM 调用，实测见 `control_at` 的日志），
//!    所以调用方要节流；这里只保证"一次调用一次返回"。
//! 2. **要过滤掉整窗级元素**：UIA 在窗口空白处返回的是 Window / Pane 元素，
//!    它们的矩形就是窗口矩形——那种情况返回 `None`，让调用方退回窗口级吸附
//!    （不然"吸到控件"和"吸到窗口"会互相打架，还会把窗口边框算丢）。
//! 3. **COM 套间**：`ElementFromPoint` 必须在初始化过 COM 的线程上调用。
//!    本模块自己保证（见 `ensure_com`），谁调用都行。

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::OnceLock;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, UIA_CONTROLTYPE_ID, UIA_PaneControlTypeId,
    UIA_TitleBarControlTypeId, UIA_WindowControlTypeId,
};

use crate::geom::Rect;

/// 小到没意义的矩形（比如 1×1 的不可见占位控件）不参与吸附。
const MIN_CONTROL_SIZE: i32 = 6;

thread_local! {
    /// 本线程是否已经初始化过 COM（`CoInitializeEx` 重复调用会返回 RPC_E_CHANGED_MODE，
    /// 那不该当成失败——线程可能已经被别的东西初始化过了）。
    static COM_READY: Cell<bool> = const { Cell::new(false) };
    /// 每个线程一份 `IUIAutomation`（COM 对象不跨套间随便传，按线程缓存最省事）。
    static AUTOMATION: OnceLock<Option<IUIAutomation>> = const { OnceLock::new() };
}

/// 已知不可用的标记：第一次发现拿不到 UIA 就不再反复尝试。
static UNAVAILABLE: Mutex<Option<String>> = Mutex::new(None);

fn automation() -> Option<IUIAutomation> {
    if let Ok(guard) = UNAVAILABLE.lock()
        && guard.is_some()
    {
        return None;
    }
    AUTOMATION.with(|slot| {
        slot.get_or_init(|| {
            COM_READY.with(|ready| {
                if !ready.get() {
                    // SAFETY: 只是初始化本线程的 COM 套间；已经初始化过会返回
                    // RPC_E_CHANGED_MODE / S_FALSE，都不影响后面创建对象（同一个进程里
                    // 只要有任意套间就行，UIA 两种套间都能用）。
                    unsafe {
                        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    }
                    ready.set(true);
                }
            });
            // SAFETY: 标准 COM 创建；失败就记一笔并返回 None。
            match unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) } {
                Ok(automation) => Some(automation),
                Err(err) => {
                    log::warn!("UI Automation 不可用（{err}），控件级吸附关闭");
                    if let Ok(mut guard) = UNAVAILABLE.lock() {
                        *guard = Some(err.to_string());
                    }
                    None
                }
            }
        })
        .clone()
    })
}

/// 逐层下潜的深度上限（病态的树不至于把我们拖住）。
const MAX_TREE_DEPTH: usize = 12;

/// 找"遮罩底下"那个顶层窗口：从 z 序最上层往下走，跳过我们自己的窗口、不可见的、
/// 最小化的、以及不含这个点的。
fn window_under_overlay(point: crate::geom::Point) -> Option<HWND> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetTopWindow, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        GW_HWNDNEXT,
    };

    let my_pid = std::process::id();
    // SAFETY: 下面全是只读的窗口查询；句柄每轮都来自系统，可能失效，失败就跳过。
    unsafe {
        let mut current = GetTopWindow(None).ok()?;
        while !current.0.is_null() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(current, Some(&mut pid));
            if pid != my_pid && IsWindowVisible(current).as_bool() && !IsIconic(current).as_bool() {
                let mut rect = RECT::default();
                if GetWindowRect(current, &mut rect).is_ok()
                    && point.x >= rect.left
                    && point.x < rect.right
                    && point.y >= rect.top
                    && point.y < rect.bottom
                {
                    return Some(current);
                }
            }
            current = match GetWindow(current, GW_HWNDNEXT) {
                Ok(next) if !next.0.is_null() => next,
                _ => break,
            };
        }
    }
    None
}

/// 在 `element` 的**直接子元素**里找"矩形包含该点、面积最小"的那一个。
fn deepest_child_at(
    automation: &IUIAutomation,
    element: &IUIAutomationElement,
    point: crate::geom::Point,
) -> Option<IUIAutomationElement> {
    use windows::Win32::UI::Accessibility::TreeScope_Children;

    // SAFETY: `FindAll` 只读；返回的元素数组由 windows crate 管理生命周期。
    let condition = unsafe { automation.CreateTrueCondition() }.ok()?;
    let children = unsafe { element.FindAll(TreeScope_Children, &condition) }.ok()?;
    let count = unsafe { children.Length() }.unwrap_or(0);
    let mut best: Option<(i64, IUIAutomationElement)> = None;
    for index in 0..count {
        let Ok(child) = (unsafe { children.GetElement(index) }) else {
            continue;
        };
        let Ok(rect) = (unsafe { child.CurrentBoundingRectangle() }) else {
            continue;
        };
        if point.x < rect.left || point.x >= rect.right || point.y < rect.top || point.y >= rect.bottom
        {
            continue;
        }
        let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
        if best.as_ref().is_none_or(|(best_area, _)| area < *best_area) {
            best = Some((area, child));
        }
    }
    best.map(|(_, child)| child)
}

/// 光标下那个控件的屏幕矩形；没有合适的控件（空白处 / 就是整窗 / 太小）时返回 `None`。
/// 返回的坐标与截图一致：**全局物理像素**。
///
/// ## 为什么不用 `ElementFromPoint`
///
/// 因为它按 z 序做命中测试，而**我们的遮罩本身就是最上层的全屏窗口**——于是它每次都返回
/// "遮罩自己"（实测：类型 50032 = Window、矩形 (0,0)-(1920,1080)），控件永远探测不到。
/// （这不是 bug，是命中测试的定义。）
///
/// 正确的做法分两步：
///   1. 自己按 z 序找出**遮罩底下**那个顶层窗口（跳过我们自己的窗口）；
///   2. 用 `ElementFromHandle` 拿到它的 UIA 元素，再**逐层往下**走，
///      每层只找"矩形包含光标"的那个子元素，直到没有更深的。
///
/// 逐层走（而不是一次 `FindAll(Descendants)`）是刻意的：浏览器那种窗口的直接后代
/// 可能上千个，一次性枚举又慢又浪费；逐层走每层只有个位数个元素，实测几毫秒。
pub fn control_at(point: crate::geom::Point) -> Option<Rect> {
    let automation = automation()?;
    let started = std::time::Instant::now();
    let hwnd = match window_under_overlay(point) {
        Some(hwnd) => hwnd,
        None => {
            log::debug!("({},{}) 底下没有别人家的窗口", point.x, point.y);
            return None;
        }
    };
    // SAFETY: `ElementFromHandle` 只读；返回的元素对象由 windows crate 管理生命周期。
    let element = match unsafe { automation.ElementFromHandle(hwnd) } {
        Ok(element) => element,
        Err(err) => {
            log::debug!("ElementFromHandle 失败（{err}）");
            return None;
        }
    };
    // 逐层下潜：每层挑"包含光标且面积最小"的那个子元素
    let mut element = element;
    for _ in 0..MAX_TREE_DEPTH {
        let Some(child) = deepest_child_at(&automation, &element, point) else {
            break;
        };
        element = child;
    }
    // SAFETY: 下面三个都是只读属性查询。
    let control_type: UIA_CONTROLTYPE_ID =
        unsafe { element.CurrentControlType() }.unwrap_or(UIA_CONTROLTYPE_ID(0));
    let offscreen = unsafe { element.CurrentIsOffscreen() }
        .map(|flag| flag.as_bool())
        .unwrap_or(false);
    let bounds = match unsafe { element.CurrentBoundingRectangle() } {
        Ok(rect) => rect,
        Err(err) => {
            log::debug!("取控件矩形失败（{err}）");
            return None;
        }
    };
    let elapsed = started.elapsed();
    log::debug!(
        "控件探测：({},{}) → 类型 {} 矩形 ({},{})-({},{}) 耗时 {:.1} ms",
        point.x,
        point.y,
        control_type.0,
        bounds.left,
        bounds.top,
        bounds.right,
        bounds.bottom,
        elapsed.as_secs_f64() * 1000.0
    );

    // 窗口/面板/标题栏是"窗口的一部分"，不是用户想单独截的那个控件：
    // 吸到标题栏那条窄边毫无意义，反而把"点一下选整窗"这个顺手的操作弄坏了。
    if offscreen
        || control_type == UIA_WindowControlTypeId
        || control_type == UIA_PaneControlTypeId
        || control_type == UIA_TitleBarControlTypeId
    {
        return None;
    }
    let w = (bounds.right - bounds.left) as u32;
    let h = (bounds.bottom - bounds.top) as u32;
    if w < MIN_CONTROL_SIZE as u32 || h < MIN_CONTROL_SIZE as u32 {
        return None;
    }
    Some(Rect::new(bounds.left, bounds.top, w, h))
}

/// 控件级吸附是否可用（拿不到 UIA 时为 `false`，设置界面据此提示）。
#[must_use]
pub fn available() -> bool {
    automation().is_some()
}

#[cfg(test)]
mod tests {
    /// 真机烟雾测试：拿桌面左上角探一下，只要求"不 panic、耗时可见"。
    ///
    /// 不硬断言"一定探到控件"——桌面那个点上可能是图标、可能是墙纸（返回整窗级元素，
    /// 我们过滤成 `None`），两种都正常。
    #[test]
    #[ignore = "需要交互式桌面；只在手动验证 UIA 通道时跑"]
    fn control_probe_smoke() {
        let start = std::time::Instant::now();
        let rect = super::control_at(crate::geom::Point::new(20, 20));
        println!(
            "control_at(20,20) = {rect:?}（首次调用含 COM 初始化，共 {:.1} ms）",
            start.elapsed().as_secs_f64() * 1000.0
        );
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let rect = super::control_at(crate::geom::Point::new(600, 400));
            println!(
                "control_at(600,400) = {rect:?} · {:.1} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
