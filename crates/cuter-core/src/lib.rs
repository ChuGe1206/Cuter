//! # cuter-core
//!
//! Cuter 的平台无关核心层。**不依赖任何 GUI 库**，因此可以被 CLI、测试或未来的
//! 插件直接复用。
//!
//! 分层约定：
//!
//! | 层 | crate | 职责 |
//! |----|-------|------|
//! | 核心 | `cuter-core` | 几何、位图运算、屏幕捕获、配置持久化 |
//! | 界面 | `cuter-ui` | egui 遮罩 / 标注 / 贴图窗口（依赖 `core`） |
//! | 应用 | `cuter-app` | 热键、托盘、窗口编排、生命周期（依赖 `ui`） |
//!
//! 所有屏幕坐标一律为 **物理像素**，坐标系为 **虚拟桌面坐标系**（左上角屏幕的
//! 左上角为原点可能为负值）。逻辑坐标只在 egui 交互的最外层出现一次。

pub mod capture;
pub mod config;
pub mod error;
pub mod frame;
pub mod geom;
pub mod history;
pub mod ocr;
pub mod scroll;
pub mod update;

pub use capture::{
    DesktopShot, MonitorInfo, ScreenCapture, SnapWindow, WindowDetector, desktop_capture,
    window_detector,
};
pub use error::{Error, Result};
pub use frame::Frame;
pub use geom::{Point, Rect, Size};
pub use history::{HistoryEntry, HistoryStore};
pub use scroll::{MatchOptions, MatchResult, PushOutcome, ScrollStitcher};
pub use update::{UpdateManifest, is_newer};

/// Cuter 版本号。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
