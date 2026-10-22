//! 屏幕捕获抽象层。
//!
//! 上层（遮罩 UI、贴图、CLI）只依赖 [`ScreenCapture`] trait 与 [`DesktopShot`]，
//! 因此新增平台后端（macOS `CGDisplayCreateImage`、Linux X11/Wayland）不需要改动
//! 任何 UI 代码。
//!
//! 坐标系约定：**虚拟桌面物理像素**。若最左侧显示器位于主屏左侧，它的 `x` 会是
//! 负数；[`DesktopShot`] 内部的 `Frame` 则以虚拟桌面左上角为 `(0, 0)`，
//! 用 [`DesktopShot::to_global`] / [`DesktopShot::to_local`] 做换算。

use crate::error::Result;
use crate::frame::Frame;
use crate::geom::{Point, Rect};

#[cfg(windows)]
mod dxgi;

/// 控件级吸附（UI Automation）。
#[cfg(windows)]
pub mod uia;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
mod win_windows;

#[cfg(windows)]
mod windows_dxgi;

pub mod cursor;

/// 捕获后端的选择（`capture.backend`）。
///
/// 默认是 `gdi`：DXGI 更快，但它依赖桌面复制（RDP 会话没有、被别的录屏程序独占时也没有），
/// 而**默认值必须是最不容易出事的那一个**。想要更低的延迟就把这里改成 `dxgi`，
/// 拿不到复制通道时会自动回退 GDI 并留日志。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    /// GDI `BitBlt`：兼容性最好，实测整屏 33.3 ms。
    #[default]
    Gdi,
    /// DXGI 桌面复制：拿 DWM 合成好的表面，预期 10~15 ms。
    Dxgi,
}

impl CaptureBackend {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Gdi => "GDI（BitBlt，兼容性最好）",
            Self::Dxgi => "DXGI 桌面复制（更快，需要系统支持）",
        }
    }
}

/// 在创建任何窗口之前把进程设置为 Per-Monitor-V2 DPI 感知。
///
/// 必须在 `eframe` 初始化之前调用，否则 Windows 会对窗口做位图拉伸，冻结截图与
/// 屏幕像素将不再一一对应（在 125% / 150% 缩放下表现最明显）。重复调用是安全的。
#[cfg(windows)]
pub use windows::ensure_dpi_awareness;

/// 一个显示器的静态信息。
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    /// 平台相关的稳定标识（Windows 上为 `hMonitor`）。
    pub id: u64,
    /// 设备名，例如 `\\.\DISPLAY1`。
    pub name: String,
    /// 显示器在虚拟桌面中的物理像素范围（含任务栏区域）。
    pub bounds: Rect,
    /// 去掉任务栏后的可用区域。
    pub work_area: Rect,
    /// 该显示器的 DPI 缩放（1.0 = 100%）。
    pub scale_factor: f32,
    pub is_primary: bool,
}

impl MonitorInfo {
    /// 逻辑尺寸（用于在设置界面展示）。
    #[must_use]
    pub fn logical_size(&self) -> (f32, f32) {
        (
            self.bounds.w as f32 / self.scale_factor,
            self.bounds.h as f32 / self.scale_factor,
        )
    }
}

/// 一次「整个虚拟桌面」的冻结截图及其元信息。
#[derive(Clone, Debug)]
pub struct DesktopShot {
    /// 覆盖全部显示器的位图，左上角对应 `bounds.origin()`。
    pub frame: Frame,
    /// 虚拟桌面在全局物理坐标中的范围。
    pub bounds: Rect,
    pub monitors: Vec<MonitorInfo>,
    /// 捕获瞬间的鼠标位置（全局物理坐标）。
    pub cursor: Option<Point>,
}

impl DesktopShot {
    /// 屏幕左上角（全局坐标）在 `frame` 中的局部坐标恒为 `(0, 0)`，
    /// 因此换算就是一次平移。
    #[must_use]
    pub fn to_local(&self, global: Point) -> Point {
        Point::new(global.x - self.bounds.x, global.y - self.bounds.y)
    }

    #[must_use]
    pub fn to_global(&self, local: Point) -> Point {
        Point::new(local.x + self.bounds.x, local.y + self.bounds.y)
    }

    /// 全局矩形 → 局部矩形。
    #[must_use]
    pub fn rect_to_local(&self, global: Rect) -> Rect {
        global.translate(-self.bounds.x, -self.bounds.y)
    }

    /// 全局矩形 → 局部矩形。
    #[must_use]
    pub fn rect_to_global(&self, local: Rect) -> Rect {
        local.translate(self.bounds.x, self.bounds.y)
    }

    /// 按全局坐标裁剪出一块结果图（自动 clamp 到桌面范围）。
    #[must_use]
    pub fn crop_global(&self, global: Rect) -> Frame {
        self.frame.crop(self.rect_to_local(global))
    }

    /// 命中所属显示器（用于取该屏的 DPI 与工作区）。
    #[must_use]
    pub fn monitor_at(&self, global: Point) -> Option<&MonitorInfo> {
        self.monitors
            .iter()
            .find(|m| m.bounds.contains(global))
            .or_else(|| {
                self.monitors
                    .iter()
                    .find(|m| m.bounds.intersect(Rect::new(global.x, global.y, 1, 1)).area() > 0)
            })
    }

    #[must_use]
    pub fn primary_monitor(&self) -> Option<&MonitorInfo> {
        self.monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| self.monitors.first())
    }

    /// 含鼠标那台显示器上的光标全局坐标。
    #[must_use]
    pub fn cursor_local(&self) -> Option<Point> {
        self.cursor.map(|c| self.to_local(c))
    }
}

/// 平台捕获后端。
///
/// 实现必须可跨线程共享：冻结截图时会先在后台线程完成捕获，再唤醒 UI。
pub trait ScreenCapture: Send + Sync {
    /// 枚举全部显示器。
    fn monitors(&self) -> Result<Vec<MonitorInfo>>;

    /// 捕获整个虚拟桌面（一次性拿到所有显示器拼合图）。
    fn capture_desktop(&self) -> Result<DesktopShot>;

    /// 只捕获指定全局区域（用于定时刷新、滚动截屏的增量帧）。
    fn capture_region(&self, region: Rect) -> Result<Frame>;

    /// 后端名称，用于日志与关于页面。
    fn backend_name(&self) -> &'static str;
}

/// 由若干显示器计算虚拟桌面范围。
#[must_use]
pub fn virtual_bounds(monitors: &[MonitorInfo]) -> Rect {
    monitors
        .iter()
        .fold(Rect::ZERO, |acc, m| acc.union(m.bounds))
}

// ---------------------------------------------------------------------------
// 窗口枚举（框选吸附用）
// ---------------------------------------------------------------------------

/// 一个可供框选吸附的顶层窗口。
///
/// 坐标一律是**全局物理像素**，与 [`DesktopShot::bounds`] 同一坐标系。
#[derive(Clone, Debug, PartialEq)]
pub struct SnapWindow {
    /// 窗口矩形。Windows 上取 DWM 扩展边框（`DWMWA_EXTENDED_FRAME_BOUNDS`），
    /// 这样吸附到的是肉眼看到的窗口边缘，而不是含不可见阴影的外框。
    pub bounds: Rect,
    /// 窗口标题（用于提示与调试）。
    pub title: String,
    /// 工具窗口 / 无标题窗口：仍然参与吸附，但排序时排在后面。
    pub is_tool: bool,
}

impl SnapWindow {
    /// 窗口面积，用于在多个候选里挑更"贴身"的那个。
    #[must_use]
    pub fn area(&self) -> u64 {
        self.bounds.area()
    }
}

/// 顶层窗口枚举器。
///
/// 之所以单独抽一层 trait：吸附逻辑是纯几何、可以完整无头测试的，
/// 平台相关的部分只有"把窗口矩形列出来"这一件事。
pub trait WindowDetector: Send + Sync {
    /// 按 z 序（最上层优先）返回可见窗口。
    ///
    /// 实现应当过滤掉：不可见窗口、最小化窗口、`WS_EX_TOOLWINDOW` 之外还需要剔除
    /// 的 shell 窗口、以及与本进程无关的空矩形。失败时返回 `Err`，调用方会退化成
    /// "只有屏幕边缘可以吸附"，而不是整个截图流程失败。
    fn visible_windows(&self) -> Result<Vec<SnapWindow>>;
}

/// 创建当前平台的窗口枚举器。
pub fn window_detector() -> Result<Box<dyn WindowDetector>> {
    #[cfg(windows)]
    {
        Ok(Box::new(win_windows::WindowsWindowDetector::new()))
    }
    #[cfg(not(windows))]
    {
        Err(crate::error::Error::Unsupported(
            "当前平台尚无窗口枚举实现（框选吸附退化为仅屏幕边缘）",
        ))
    }
}

/// 创建指定平台的捕获后端。
///
/// `backend` 来自 `capture.backend` 配置。**不做静默回退**：拿不到就直接报错，
/// 由调用方决定怎么办——回退是应用层的策略（`app::desktop_backend`），
/// 而且要留一条日志说明"这次用的是哪个后端"，否则"性能没变化"会永远查不出原因。
pub fn desktop_capture_with(backend: CaptureBackend) -> Result<Box<dyn ScreenCapture>> {
    #[cfg(windows)]
    {
        match backend {
            CaptureBackend::Gdi => Ok(Box::new(windows::WindowsCapture::new()?)),
            CaptureBackend::Dxgi => Ok(Box::new(windows_dxgi::DxgiScreenCapture::new()?)),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = backend;
        Err(crate::error::Error::Unsupported(
            "当前平台尚无捕获后端（macOS / Linux 后端规划中）",
        ))
    }
}

/// 创建当前平台的默认捕获后端（GDI）。
pub fn desktop_capture() -> Result<Box<dyn ScreenCapture>> {
    desktop_capture_with(CaptureBackend::Gdi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;

    fn fake_shot() -> DesktopShot {
        let monitors = vec![
            MonitorInfo {
                id: 1,
                name: "\\\\.\\DISPLAY1".into(),
                bounds: Rect::new(-1920, 0, 1920, 1080),
                work_area: Rect::new(-1920, 0, 1920, 1040),
                scale_factor: 1.0,
                is_primary: false,
            },
            MonitorInfo {
                id: 2,
                name: "\\\\.\\DISPLAY2".into(),
                bounds: Rect::new(0, 0, 2560, 1440),
                work_area: Rect::new(0, 0, 2560, 1400),
                scale_factor: 1.25,
                is_primary: true,
            },
        ];
        DesktopShot {
            frame: Frame::new(Size::new(4480, 1440)),
            bounds: virtual_bounds(&monitors),
            monitors,
            cursor: Some(Point::new(100, 100)),
        }
    }

    #[test]
    fn virtual_bounds_unions_monitors_with_negative_origin() {
        let shot = fake_shot();
        assert_eq!(shot.bounds, Rect::new(-1920, 0, 4480, 1440));
    }

    #[test]
    fn global_and_local_conversions_roundtrip() {
        let shot = fake_shot();
        let global = Point::new(-100, 50);
        assert_eq!(shot.to_local(global), Point::new(1820, 50));
        assert_eq!(shot.to_global(shot.to_local(global)), global);

        let rect = Rect::new(-500, 10, 100, 100);
        assert_eq!(shot.rect_to_local(rect), Rect::new(1420, 10, 100, 100));
        assert_eq!(shot.rect_to_global(shot.rect_to_local(rect)), rect);
    }

    #[test]
    fn monitor_at_finds_left_display() {
        let shot = fake_shot();
        let m = shot.monitor_at(Point::new(-10, 10)).unwrap();
        assert_eq!(m.id, 1);
        assert_eq!(shot.monitor_at(Point::new(10, 10)).unwrap().id, 2);
        assert!(shot.primary_monitor().unwrap().is_primary);
    }
}
