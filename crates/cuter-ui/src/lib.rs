//! # cuter-ui
//!
//! Cuter 的界面层。只依赖 `egui` / `eframe` 与 `cuter-core`，不触碰任何平台 API，
//! 因此可以独立编译、独立做无头单元测试。
//!
//! 模块职责：
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`theme`] | 配色与视觉常量 |
//! | [`fonts`] | 系统中文字体探测与装载（egui 自带字体没有 CJK 字形） |
//! | [`annotate`] | 标注数据模型、文档、撤回栈、egui 绘制 |
//! | [`snap`] | 框选吸附的纯几何（窗口/屏幕的边缘与中线） |
//! | [`flatten`] | 把标注真正画进像素（导出 PNG / 复制到剪贴板） |
//! | [`toolbar`] | 浮动工具条：工具、颜色、线宽、撤销、复制、保存、贴图 |
//! | [`overlay`] | 全屏冻结遮罩：框选、吸附、放大镜、取色、快捷键 |
//! | [`pin`] | 屏幕贴图窗口的内容绘制 |
//! | [`ocr_panel`] | OCR 识别结果面板（左图右文，可校对后复制） |
//! | [`scroll_panel`] | 滚动截屏控制面板（实时缩略长图 + 完成/暂停/取消） |
//! | [`history_panel`] | 截图历史面板（缩略图 + 再复制 / 再贴图 / 另存为 / 删除） |

pub mod annotate;
pub mod flatten;
pub mod fonts;
pub mod frost;
pub mod i18n;
pub mod history_panel;
pub mod ocr_panel;
pub mod overlay;
pub mod pin;
pub mod scroll_panel;
pub mod snap;
pub mod theme;
pub mod toolbar;
pub mod tray_panel;
