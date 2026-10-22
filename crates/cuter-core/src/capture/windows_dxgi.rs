//! 把 [`DxgiCapture`] 接到 [`ScreenCapture`] trait 上。
//!
//! 分工很明确：
//!
//! * **像素**来自 DXGI 桌面复制（`dxgi.rs`）；
//! * **显示器元信息**（工作区、DPI、主屏标记）仍然问 GDI 那一套
//!   （`WindowsCapture::monitors`，实测 0.00 ms，而且它已经处理好了 DPI 与工作区）；
//! * **光标位置**用既有的 `GetCursorPos` 路径。
//!
//! 这样新增一个后端不需要重写任何元信息逻辑，也让两种后端给出的
//! `DesktopShot` 结构完全一致——上层（遮罩 / CLI / 探针）感知不到区别。

use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::geom::{Point, Rect};

use super::dxgi::DxgiCapture;
use super::windows::WindowsCapture;
use super::{DesktopShot, MonitorInfo, ScreenCapture, virtual_bounds};

/// DXGI 捕获后端。
pub struct DxgiScreenCapture {
    dxgi: DxgiCapture,
    /// 只用来枚举显示器与取光标位置。
    gdi: WindowsCapture,
}

impl DxgiScreenCapture {
    /// 建立 DXGI 复制通道；**任何一步失败都返回错误**，由应用层回退 GDI。
    pub fn new() -> Result<Self> {
        let gdi = WindowsCapture::new()?;
        let dxgi = DxgiCapture::new()?;
        let capture = Self { dxgi, gdi };

        // 把覆盖到的输出打出来：出问题时这是第一手证据（哪个显示器没建起来）
        log::info!(
            "DXGI 桌面复制后端就绪，覆盖 {} 个输出",
            capture.output_count()
        );
        for (name, bounds) in capture.dxgi.output_list() {
            log::info!(
                "DXGI 输出：{name} {}×{} @({},{})",
                bounds.w,
                bounds.h,
                bounds.x,
                bounds.y
            );
        }
        // 交叉核对一下：DXGI 报告的显示器矩形应当覆盖 GDI 报的虚拟桌面，
        // 不一致说明有输出没建起来（混合 DPI / 某个输出被独占），值得留一条日志。
        let gdi_bounds = virtual_bounds(&capture.gdi.monitors()?);
        let dxgi_bounds = capture.dxgi.virtual_bounds();
        if gdi_bounds != dxgi_bounds {
            log::warn!(
                "DXGI 覆盖范围 {}x{}+{}+{} 与 GDI 报的虚拟桌面 {}x{}+{}+{} 不一致，截图可能不完整",
                dxgi_bounds.w,
                dxgi_bounds.h,
                dxgi_bounds.x,
                dxgi_bounds.y,
                gdi_bounds.w,
                gdi_bounds.h,
                gdi_bounds.x,
                gdi_bounds.y
            );
        }
        Ok(capture)
    }

    /// 建立了复制通道的输出数量。
    ///
    /// 也是 [`ScreenCapture::backend_name`] 之外唯一对外暴露的诊断信息：
    /// 探针/日志用它确认"DXGI 真的接上了"，而不是悄悄回退到 GDI。
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.dxgi.output_count()
    }
}

impl ScreenCapture for DxgiScreenCapture {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        self.gdi.monitors()
    }

    fn capture_desktop(&self) -> Result<DesktopShot> {
        let monitors = self.monitors()?;
        let bounds = virtual_bounds(&monitors);
        let frame = self
            .dxgi
            .grab(bounds)
            .map_err(|err| wrap("DXGI 抓取整个虚拟桌面失败", err))?;
        Ok(DesktopShot {
            frame,
            bounds,
            monitors,
            cursor: cursor_position(),
        })
    }

    fn capture_region(&self, region: Rect) -> Result<Frame> {
        self.dxgi
            .grab(region)
            .map_err(|err| wrap("DXGI 抓取区域失败", err))
    }

    fn backend_name(&self) -> &'static str {
        "Windows DXGI（桌面复制）"
    }
}

/// 光标位置（物理像素；取不到就是 `None`，与 GDI 后端保持一致）。
fn cursor_position() -> Option<Point> {
    let mut point = windows::Win32::Foundation::POINT::default();
    // SAFETY: `GetCursorPos` 只写一个栈上的 POINT。
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return None;
    }
    Some(Point::new(point.x, point.y))
}

fn wrap(what: &'static str, err: Error) -> Error {
    match err {
        // `Unsupported` 是"这次环境不行"（超时 / ACCESS_LOST），原样往上抛，
        // 让应用的日志里能看到具体原因。
        Error::Unsupported(reason) => Error::Unsupported(reason),
        other => {
            log::warn!("{what}：{other}");
            other
        }
    }
}
