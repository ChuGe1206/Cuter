//! 全屏冻结遮罩。
//!
//! ## 一次截图的生命周期
//!
//! ```text
//! 热键
//!  └─ 后台线程捕获整个虚拟桌面（此时屏幕还没变，用户看不到任何闪烁）
//!      └─ 覆盖虚拟桌面的无边框窗口把冻结帧画上去
//!          ├─ 空闲：十字辅助线 + 放大镜 + 取色
//!          ├─ 拖拽框选：实时尺寸/坐标 + 八向控制点
//!          ├─ 标注：矩形/椭圆/箭头/画笔/荧光笔/马赛克/模糊/文字/序号
//!          └─ 完成：复制 / 保存 / 贴图 / OCR → 返回 [`OverlayResult`]
//! ```
//!
//! ## 坐标系统一：为什么强制 `pixels_per_point = 1.0`
//!
//! 遮罩窗口覆盖整个虚拟桌面，尺寸就是虚拟桌面的**物理像素**尺寸。只要把 egui 的
//! `pixels_per_point` 固定为 1.0，egui 的「点」就与屏幕物理像素一一对应，于是：
//!
//! * 鼠标位置不需要任何换算，直接就是像素坐标；
//! * 冻结帧 1:1 贴到窗口上，不会因为系统缩放被重采样（125%/150% 下「截图比实际
//!   模糊」的最常见根因就是这个）；
//! * 多显示器混合 DPI 时，不会因为窗口跨越不同缩放率的屏幕而让坐标漂移。
//!
//! 代价是界面外壳（工具条）不随系统缩放放大——这是刻意取舍：**截图内容必须精确**。
//!
//! ## 框选吸附
//!
//! 参考线由宿主传入的窗口列表（`cuter_core::capture::WindowDetector`）与屏幕自身
//! 生成，吸附本身是纯几何，见 [`crate::snap`]。**按住 Alt 可临时关闭吸附**，
//! 这是所有主流截图工具的约定，用来处理"就是要差一像素"的场景。

use crate::{fill, tr};

use egui::{
    Align2, Color32, CursorIcon, Id, Key, LayerId, Order, Painter, Pos2, Rect as ERect, Stroke,
    StrokeKind, Vec2,
};

use cuter_core::capture::{DesktopShot, SnapWindow};
use cuter_core::config::{CaptureConfig, Config, ToolKind};
use cuter_core::frame::Frame;
use cuter_core::geom::{Point, Rect, Size};

use crate::annotate::{Annotation, Document, ToolSettings};
use crate::flatten;
use crate::fonts::LoadedFont;
use crate::snap::{self, MovingEdges, SnapOutcome, SnapTargets};

/// 控件探测的节流间隔：`ElementFromPoint` 一次 5~8 ms，别每帧都问。
const CONTROL_PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);
/// 指针移动超过这么多像素才重新问一次（同一个控件里晃来晃去不必重复问）。
const CONTROL_PROBE_TOLERANCE: f32 = 3.0;

/// 上一次控件探测的结果与时机。
struct ControlProbe {
    at: Pos2,
    queried_at: std::time::Instant,
    rect: Option<ERect>,
}
use crate::theme::{Theme, metrics};
use crate::toolbar::{self, ToolbarAction, ToolbarState};

/// 小于这个尺寸的框选视为误触。
const MIN_SELECTION: u32 = 4;
/// 按下到松开之间移动不超过这么多 egui 点，就算"单击"而不是"拖拽"。
/// 4 点（≈ 5 物理像素 @125%）足够容忍手抖，又远小于任何有意义的框选。
const CLICK_SLOP: f32 = 4.0;

/// 整体变暗的乘算色调（明亮度约 45%）。
const DIM_TINT: Color32 = Color32::from_gray(115);

/// 双击判定：两次按下间隔上限（秒）与位移上限（像素）。
const DOUBLE_CLICK_SECS: f64 = 0.35;
const DOUBLE_CLICK_SLOP: f32 = 6.0;

/// 遮罩结束时要执行的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAction {
    Copy,
    Save,
    SaveAs,
    Pin,
    Ocr,
    /// 用户放弃（Esc / 右键 / 关闭按钮）。
    Cancel,
    /// 重新捕获一帧。
    Recapture,
    OpenSettings,
    /// 用当前选区开始滚动截屏（把首帧交给宿主，由它去持续采集）。
    ScrollCapture,
}

/// 遮罩的产出：动作 + 已经烘焙好的图片 + 图片在桌面上的全局位置。
#[derive(Debug)]
pub struct OverlayResult {
    pub action: OverlayAction,
    /// `Cancel` / `Recapture` / `OpenSettings` 时为 `None`。
    pub frame: Option<Frame>,
    /// 结果图左上角的全局物理坐标（贴图定位用）。
    pub origin: Point,
    /// 最终选区的全局矩形（宿主用它做「区域记忆」）。
    /// 没有选区时为 [`Rect::ZERO`]。
    pub region: Rect,
}

/// 八个方向的控制点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    const ALL: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    fn position(self, rect: ERect) -> Pos2 {
        match self {
            Self::NW => rect.left_top(),
            Self::N => rect.center_top(),
            Self::NE => rect.right_top(),
            Self::E => rect.right_center(),
            Self::SE => rect.right_bottom(),
            Self::S => rect.center_bottom(),
            Self::SW => rect.left_bottom(),
            Self::W => rect.left_center(),
        }
    }

    fn moves_left(self) -> bool {
        matches!(self, Self::NW | Self::W | Self::SW)
    }
    fn moves_right(self) -> bool {
        matches!(self, Self::NE | Self::E | Self::SE)
    }
    fn moves_top(self) -> bool {
        matches!(self, Self::NW | Self::N | Self::NE)
    }
    fn moves_bottom(self) -> bool {
        matches!(self, Self::SW | Self::S | Self::SE)
    }

    fn cursor(self) -> CursorIcon {
        match self {
            Self::NW | Self::SE => CursorIcon::ResizeNwSe,
            Self::NE | Self::SW => CursorIcon::ResizeNeSw,
            Self::N | Self::S => CursorIcon::ResizeVertical,
            Self::E | Self::W => CursorIcon::ResizeHorizontal,
        }
    }
}

/// 当前拖拽状态。
#[derive(Clone, Debug, PartialEq)]
enum Drag {
    None,
    /// 正在拉出新选区（遮罩局部坐标）。
    Create { anchor: Pos2 },
    /// 正在平移已有选区。
    Move { grab: Pos2, original: ERect },
    /// 正在拉伸控制点。
    Resize { handle: Handle, original: ERect },
    /// 正在用标注工具绘制（文档坐标）。
    Annotate { anchor: Pos2 },
}

/// 正在进行中的文字输入。
#[derive(Clone, Debug)]
struct TextInput {
    /// 文档坐标。
    pos: Pos2,
    buffer: String,
    /// 首帧需要抢焦点。
    fresh: bool,
}

/// 全屏冻结遮罩。
pub struct Overlay {
    /// 冻结帧文档（`base` 就是整张桌面帧，见 [`crate::annotate`] 的坐标说明）。
    doc: Document,
    /// 虚拟桌面在全局坐标中的范围。
    bounds: Rect,
    theme: Theme,
    settings: ToolSettings,
    toolbar: ToolbarState,
    drag: Drag,
    /// 选区（遮罩局部坐标 == 桌面帧局部坐标）。
    selection: Option<Rect>,
    cursor: Option<Pos2>,
    /// 上一次主键按下的时间与位置，用于自己实现双击判定。
    last_click: Option<(f64, Pos2)>,
    pending_annotation: Option<Annotation>,
    pending_effect: Option<Rect>,
    text_input: Option<TextInput>,
    font: Option<LoadedFont>,
    magnifier: bool,
    /// 提示文字剩余显示帧数。
    status_frames: u32,
    /// 已经产生结果，等待宿主关闭窗口。
    finished: bool,
    capture_cfg: CaptureConfig,
    /// 吸附参考线（窗口与屏幕的边缘/中线，遮罩局部坐标）。
    snap_targets: SnapTargets,
    /// 用于「鼠标悬停时高亮窗口」的窗口矩形（遮罩局部坐标，按 z 序）。
    snap_rects: Vec<ERect>,
    /// 是否启用吸附（配置开关；按住 Alt 时临时关闭）。
    snap_enabled: bool,
    /// 是否启用**控件级**吸附（`capture.snap_controls`）。
    control_snap_enabled: bool,
    /// 控件探测的节流缓存（见 `control_snap_rect`）。
    control_probe: Option<ControlProbe>,
    /// 本次按下时的指针位置与当时高亮的窗口矩形（用来区分"单击选窗口"与"拖拽自框"）。
    press_pos: Option<Pos2>,
    click_target: Option<ERect>,
    /// 本帧命中的吸附参考线，用于绘制。
    active_snap: SnapOutcome,
    /// 是否按住 Alt（临时关闭吸附，用来处理"就是要差一像素"的场景）。
    alt_held: bool,
    /// Ctrl 是否按住：按住 = 拖动移**移动**已有选区；不按 = 选区内拖动是**重新框选**。
    ctrl_held: bool,
    /// 工具样式改动，等宿主取走写进配置（见 [`Overlay::take_style_change`]）。
    pending_style: Option<ToolSettings>,
    /// 配置里勾了「截图后自动保存」时，回车/双击的默认动作变成保存而不是复制。
    auto_save: bool,
    /// 配置里勾了「框选后立即识别」时，默认动作变成 OCR（这类工具的常见流程）。
    auto_ocr: bool,
    /// 本次遮罩是「滚动截屏」模式：确认后交给宿主持续采集，而不是直接出图。
    scroll_mode: bool,
    /// 上次框选的区域（全局坐标），`Ctrl+R` 复原用。
    last_region: Option<Rect>,
}

impl Overlay {
    /// 消费一次冻结捕获，构建遮罩。
    ///
    /// `snap_windows` 来自宿主在捕获前枚举的顶层窗口（全局物理坐标）；
    /// 传空数组时退化成「只能吸附屏幕边缘」。
    #[must_use]
    pub fn new(
        shot: DesktopShot,
        config: &Config,
        theme: Theme,
        font: Option<LoadedFont>,
        snap_windows: Vec<SnapWindow>,
    ) -> Self {
        let DesktopShot {
            frame,
            bounds,
            monitors: _,
            cursor,
        } = shot;

        let settings = ToolSettings::from_config(&config.annotate);
        let mut toolbar = ToolbarState {
            settings: settings.clone(),
            magnifier: config.capture.show_magnifier,
            opacity: config.pin.opacity,
            ..ToolbarState::default()
        };
        toolbar.snap_enabled = config.capture.auto_detect_windows;

        let cursor_local = cursor.map(|c| Pos2::new((c.x - bounds.x) as f32, (c.y - bounds.y) as f32));

        // 窗口矩形从全局坐标换算到遮罩局部坐标（两者只差 bounds.origin 的平移）
        let snap_rects: Vec<ERect> = snap_windows
            .iter()
            .map(|w| {
                to_egui(w.bounds.translate(-bounds.x, -bounds.y))
            })
            .collect();
        let screen = ERect::from_min_size(
            Pos2::ZERO,
            Vec2::new(bounds.w as f32, bounds.h as f32),
        );
        let snap_targets = SnapTargets::from_rects(&snap_rects, screen);

        let mut doc = Document::new(bounds.origin(), frame);
        doc.set_undo_limit(config.annotate.undo_limit);

        Self {
            doc,
            bounds,
            theme,
            settings,
            toolbar,
            drag: Drag::None,
            selection: None,
            cursor: cursor_local,
            last_click: None,
            pending_annotation: None,
            pending_effect: None,
            text_input: None,
            font,
            magnifier: config.capture.show_magnifier,
            status_frames: 0,
            finished: false,
            capture_cfg: config.capture.clone(),
            snap_targets,
            snap_rects,
            snap_enabled: config.capture.auto_detect_windows,
            control_snap_enabled: config.capture.snap_controls,
            control_probe: None,
            press_pos: None,
            click_target: None,
            active_snap: SnapOutcome::default(),
            alt_held: false,
            ctrl_held: false,
            pending_style: None,
            auto_save: config.save.auto_save,
            auto_ocr: config.ocr.auto_after_capture && config.ocr.engine != cuter_core::config::OcrEngine::Disabled,
            scroll_mode: false,
            last_region: config.capture.last_region,
        }
    }

    /// 虚拟桌面的物理尺寸。
    #[must_use]
    pub fn desktop_size(&self) -> Size {
        self.doc.size()
    }

    /// 当前选区（全局坐标）。
    #[must_use]
    pub fn selection_global(&self) -> Option<Rect> {
        self.selection
            .map(|s| s.translate(self.bounds.x, self.bounds.y))
    }

    /// 宿主是否应当关闭遮罩窗口。
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// 把当前选区 + 标注烘焙成最终图片。
    ///
    /// 标注坐标是桌面帧坐标，这里整体平移 `-选区原点`；越界部分由栅格化器自动裁剪。
    #[must_use]
    pub fn render(&self) -> Option<Frame> {
        let selection = self.selection?;
        if selection.is_empty() {
            return None;
        }
        let base = self.doc.base().crop(selection);
        if self.doc.annotations().is_empty() {
            return Some(base);
        }
        let delta = Vec2::new(-(selection.x as f32), -(selection.y as f32));
        let mut annotations: Vec<Annotation> = self.doc.annotations().to_vec();
        for annotation in &mut annotations {
            annotation.translate(delta);
        }
        let font = self.font.as_ref().map(LoadedFont::font_ref);
        Some(flatten::flatten(&base, &annotations, font))
    }

    /// 每帧驱动遮罩。返回 `Some` 表示用户已经做出决定，宿主应当处理并关闭窗口。
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<OverlayResult> {
        // 关键：让「点」与物理像素一一对应（见模块文档）
        ctx.set_pixels_per_point(1.0);
        // 遮罩必须跟随鼠标实时重绘
        ctx.request_repaint();

        let screen = ERect::from_min_size(
            Pos2::ZERO,
            Vec2::new(self.doc.size().w as f32, self.doc.size().h as f32),
        );

        if self.status_frames > 0 {
            self.status_frames -= 1;
            if self.status_frames == 0 {
                self.toolbar.status = None;
            }
        }

        self.cursor = ctx.input(|i| i.pointer.latest_pos());
        // Alt 临时关闭吸附、Ctrl 拖动移动选区：每帧读一次，避免在拖拽逻辑里反复查询输入
        self.alt_held = ctx.input(|i| i.modifiers.alt);
        self.ctrl_held = ctx.input(|i| i.modifiers.ctrl);
        self.sync_toolbar_state();

        // 上一帧是否有弹窗打开：若开着，本帧的点击只用于关弹窗，不参与画布交互
        let popup_was_open = self.toolbar.any_popup_open();
        let text_active = self.text_input.is_some();

        let mut result = None;
        if !text_active {
            result = self.handle_keys(ctx, screen);
        }
        if result.is_none() && !text_active && !popup_was_open {
            result = self.handle_pointer(ctx, screen);
        }

        if result.is_none() {
            self.paint(ctx, screen);
        }
        if result.is_none() && let Some(r) = self.text_input_ui(ctx) {
            result = Some(r);
        }

        if result.is_none()
            && let Some(selection) = self.selection
        {
            let anchor = to_egui(selection);
            // 设计稿：浮动面板底下垫一层「冻结帧模糊 + 叠白」的毛玻璃（自己算像素，不依赖系统合成器）
            let bar_size = toolbar::desired_size();
            let panel =
                ERect::from_min_size(toolbar::position(anchor, bar_size, screen), bar_size).expand(6.0);
            let full = self.doc.size();
            if !full.is_empty() {
                let uv = ERect::from_min_max(
                    Pos2::new(panel.left() / full.w as f32, panel.top() / full.h as f32),
                    Pos2::new(panel.right() / full.w as f32, panel.bottom() / full.h as f32),
                );
                let frost = self.doc.frost_texture(ctx).id();
                ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::new("cuter-frost"),
                ))
                .image(frost, panel, uv, Color32::WHITE);
            }
            if let Some(action) = toolbar::show(ctx, &mut self.toolbar, &self.theme, anchor, screen)
            {
                result = self.apply_toolbar(action);
            }
        }

        if result.is_none() {
            self.update_cursor_icon(ctx, screen);
        }

        if result.is_some() {
            self.finished = true;
        }
        result
    }

    // ------------------------------------------------------------------
    // 输入
    // ------------------------------------------------------------------

    /// 把文档状态同步到工具条，并把工具条上的参数改动收回 Overlay。
    fn sync_toolbar_state(&mut self) {
        self.toolbar.can_undo = self.doc.can_undo();
        self.toolbar.can_redo = self.doc.can_redo();
        self.toolbar.has_selection = self.selection.is_some();
        self.toolbar.magnifier = self.magnifier;
        self.toolbar.snap_enabled = self.snap_enabled;
        self.settings = self.toolbar.settings.clone();
    }

    /// 此刻是否允许吸附（配置开关 + 没有按住 Alt）。
    fn snap_allowed(&self) -> bool {
        self.snap_enabled && !self.alt_held && !self.snap_targets.is_empty()
    }

    /// `Ctrl+R`：复原上次框选的区域。
    ///
    /// 记忆里存的是**全局坐标**，而遮罩内部一律用局部坐标，所以这里要平移；
    /// 显示器布局变化过之后旧区域可能已落在屏幕外，因此还要与当前桌面求交，
    /// 求交后过小就放弃（宁可不复原，也不要给出一个看起来正确其实是错的选区）。
    fn restore_last_region(&mut self, screen: ERect) {
        let Some(global) = self.last_region else {
            self.notify(tr!("还没有可复原的选区（框选一次后会自动记住）"));
            return;
        };
        let local = global.translate(-self.bounds.x, -self.bounds.y);
        let clamped = clamp_rect(local, screen);
        if clamped.w < MIN_SELECTION || clamped.h < MIN_SELECTION {
            self.notify(tr!("上次的选区已不在当前屏幕范围内"));
            return;
        }
        self.reset_selection();
        self.selection = Some(clamped);
        self.notify(fill!(tr!("已复原上次选区 {} × {}"), clamped.w, clamped.h));
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.toolbar.status = Some(text.into());
        self.status_frames = 90;
    }

    /// 切换为「滚动截屏」模式：确认选区后交给宿主持续采集，而不是直接出图。
    ///
    /// 进入该模式时会给出提示——滚动截屏的交互和普通截图差别很大（框完还要去滚页面），
    /// 不告诉用户的话他会以为按了回车就完事了。
    pub fn set_scroll_mode(&mut self, enabled: bool) {
        self.scroll_mode = enabled;
        if enabled {
            self.notify(tr!("滚动截屏：框选要截取的区域，然后按回车开始（之后正常滚动即可）"));
        }
    }

    /// 是否处于滚动截屏模式。
    #[must_use]
    pub fn scroll_mode(&self) -> bool {
        self.scroll_mode
    }

    /// 取走"工具样式被改过"这件事（宿主每帧调用一次，拿到就写回配置）。
    ///
    /// 走旁路而不是 `OverlayResult`：宿主收到任何结果都会销毁遮罩，
    /// 那条路会让"拖一下线宽滑块把遮罩关掉"。
    pub fn take_style_change(&mut self) -> Option<ToolSettings> {
        self.pending_style.take()
    }

    fn handle_keys(&mut self, ctx: &egui::Context, screen: ERect) -> Option<OverlayResult> {
        let (escape, enter, shift, ctrl) =
            ctx.input(|i| {
                (
                    i.key_pressed(Key::Escape),
                    i.key_pressed(Key::Enter),
                    i.modifiers.shift,
                    i.modifiers.ctrl,
                )
            });

        if escape {
            if self.selection.is_some() {
                // 第一次 Esc 只取消选区，第二次才退出（与主流截图工具一致）
                self.reset_selection();
                self.notify(tr!("已取消选区"));
                return None;
            }
            return Some(self.result(OverlayAction::Cancel));
        }

        if ctrl {
            let (c, s, t, o, z, y, a, r) = ctx.input(|i| {
                (
                    i.key_pressed(Key::C),
                    i.key_pressed(Key::S),
                    i.key_pressed(Key::T),
                    i.key_pressed(Key::O),
                    i.key_pressed(Key::Z),
                    i.key_pressed(Key::Y),
                    i.key_pressed(Key::A),
                    i.key_pressed(Key::R),
                )
            });
            // 没有选区时这几个快捷键什么也做不了：给出提示而不是返回一个没有图片的
            // 结果（那样宿主只会打一条 "收到没有图片的结果" 的警告，用户一脸茫然）。
            if c || s || t || o {
                if self.selection.is_none() {
                    self.notify(tr!("请先拖拽框选一个区域"));
                    return None;
                }
                let action = if c {
                    OverlayAction::Copy
                } else if s {
                    OverlayAction::Save
                } else if t {
                    OverlayAction::Pin
                } else {
                    OverlayAction::Ocr
                };
                return Some(self.result(action));
            }
            if z {
                if shift {
                    self.doc.redo();
                } else {
                    self.doc.undo();
                }
                return None;
            }
            if y {
                self.doc.redo();
                return None;
            }
            if a {
                self.selection = Some(Rect::new(0, 0, self.doc.size().w, self.doc.size().h));
                self.notify(tr!("已全选"));
                return None;
            }
            if r {
                self.restore_last_region(screen);
                return None;
            }
        }

        // 方向键微调选区
        let step = if shift { 10 } else { 1 };
        let (left, right, up, down) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
            )
        });
        if (left || right || up || down)
            && let Some(selection) = self.selection
        {
            let dx = i32::from(right) * step - i32::from(left) * step;
            let dy = i32::from(down) * step - i32::from(up) * step;
            self.selection = Some(clamp_rect(selection.translate(dx, dy), screen));
        }

        if enter && self.selection.is_some() {
            return Some(self.confirm_default());
        }

        None
    }

    /// 回车 / 双击选区的默认动作。
    ///
    /// 优先级：**滚动截屏** > **框选后立即识别** > **自动保存** > 默认的复制。
    /// 四者的取值都来自设置界面或热键，所以每个开关都能在代码里找到作用点。
    fn confirm_default(&self) -> OverlayResult {
        let action = if self.scroll_mode {
            OverlayAction::ScrollCapture
        } else if self.auto_ocr {
            OverlayAction::Ocr
        } else if self.auto_save {
            OverlayAction::Save
        } else {
            OverlayAction::Copy
        };
        self.result(action)
    }

    fn handle_pointer(&mut self, ctx: &egui::Context, screen: ERect) -> Option<OverlayResult> {
        let (pos, pressed, released, secondary, time) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.secondary_pressed(),
                i.time,
            )
        });

        let pos = pos?;
        let pos = clamp_pos(pos, screen);

        // 落在工具条上的点击交给工具条的控件处理
        if pressed && self.toolbar_rect(screen).is_some_and(|r| r.contains(pos)) {
            self.last_click = None;
            return None;
        }

        if secondary {
            if self.selection.is_some() {
                self.reset_selection();
                self.notify(tr!("已取消选区"));
            } else {
                return Some(self.result(OverlayAction::Cancel));
            }
            return None;
        }

        if pressed {
            let double = self.last_click.is_some_and(|(t, p)| {
                time - t <= DOUBLE_CLICK_SECS && p.distance(pos) <= DOUBLE_CLICK_SLOP
            });
            self.last_click = Some((time, pos));
            // 记下这次按下的原始位置与当时高亮的窗口：松手时用来判断
            // "这是一次点击（选窗口）还是一次拖拽（自己框）"。
            //
            // 什么时候允许"单击选窗口"：
            //   * 还没有选区 —— 此刻任何工具都画不了标注（标注都画在选区里），
            //     所以单击的含义只能是"就选这个窗口"；
            //   * 或者当前就是选择工具 —— 用户明确在"选"的状态。
            // 其它情况（已有选区 + 矩形/箭头等工具）单击是画标注，不能抢。
            self.press_pos = Some(pos);
            self.click_target = if self.selection.is_none() || self.settings.tool == ToolKind::Select {
                self.hovered_snap_rect(pos)
            } else {
                None
            };
            log::debug!(
                "遮罩按下：pos={pos:?} 高亮窗口={:?} 吸附可用={} 工具={:?}",
                self.click_target,
                self.snap_allowed(),
                self.settings.tool
            );

            let inside_selection = self
                .selection
                .is_some_and(|s| to_egui(s).contains(pos));
            if double && inside_selection && matches!(self.drag, Drag::None | Drag::Move { .. }) {
                // 双击选区确认（主流截图工具的通用手势）
                return Some(self.confirm_default());
            }
            self.begin_drag(pos, screen);
        }

        // 拖拽中：用自己维护的 Drag 状态判断，而不是依赖 pointer.primary_down()
        // （egui 0.36 的 PointerState 没有暴露 primary_down）
        if !released && !matches!(self.drag, Drag::None) {
            match self.drag.clone() {
                Drag::Create { .. } | Drag::Move { .. } | Drag::Resize { .. } => {
                    self.update_selection_drag(pos, screen);
                }
                Drag::Annotate { .. } => {
                    let local = self.document_pos(pos);
                    self.update_annotation_drag(local);
                }
                Drag::None => {}
            }
        }

        if released && !matches!(self.drag, Drag::None) {
            log::debug!("遮罩松开：drag={:?} press={:?}", self.drag, self.press_pos);
            self.finish_drag();
            // 松手后：如果这次其实只是"点了一下"，而且按下的地方有高亮窗口，
            // 就把那个窗口作为选区（见 `click_selects_hovered_window`）。
            self.click_selects_hovered_window(pos, screen);
        }

        None
    }

    fn begin_drag(&mut self, pos: Pos2, screen: ERect) {
        self.toolbar.close_popups();

        // 已有选区时的优先级：控制点 > 选区内部 > 选区之外
        if let Some(selection) = self.selection {
            let rect = to_egui(selection);
            if let Some(handle) = handle_at_in_screen(rect, pos, screen) {
                self.drag = Drag::Resize {
                    handle,
                    original: rect,
                };
                return;
            }
            if rect.contains(pos) {
                let local = self.document_pos(pos);
                // 选区内拖动到底是"移动选区"还是"重新框选"？
                // 定成：**默认重新框选，按住 Ctrl 才是移动**。
                // 理由：预设/整屏选区（截取当前显示器、Ctrl+R 复原）出现时，
                // 用户在它里面随手一拖本意是"重新框一块"，结果整个选区被拖着跑，
                // 看起来像"选中了却还在被拖来拖去"（HANDOFF 第 20 节记了这次反馈）。
                // 缩放仍然走八个控制点，移动还有方向键，所以不按住 Ctrl 也不会被困住。
                let handled = match self.settings.tool {
                    ToolKind::Select => {
                        if self.ctrl_held {
                            self.drag = Drag::Move {
                                grab: pos,
                                original: rect,
                            };
                            true
                        } else {
                            // 交给下面的"拉出新选区"
                            false
                        }
                    }
                    ToolKind::Text => {
                        self.text_input = Some(TextInput {
                            pos: local,
                            buffer: String::new(),
                            fresh: true,
                        });
                        self.drag = Drag::None;
                        true
                    }
                    ToolKind::Counter => {
                        let index = self.doc.take_counter_index();
                        self.doc.push_annotation(Annotation::Counter {
                            center: local,
                            index,
                            color: self.settings.color,
                            radius: self.settings.counter_radius,
                        });
                        self.drag = Drag::None;
                        true
                    }
                    tool => {
                        self.drag = Drag::Annotate { anchor: local };
                        self.pending_effect = None;
                        self.pending_annotation =
                            Some(self.make_annotation(tool, local, local, &[]));
                        true
                    }
                };
                if handled {
                    return;
                }
            }
        }

        // 拉出新选区。
        // 起点先吸附一次：用户从窗口边缘附近按下时，起点本身就对齐到那条边，
        // 否则终点吸得再准，整块选区还是会差一个起点偏移。
        let anchor = if self.snap_allowed() {
            pos + snap::snap_corner(pos, &self.snap_targets, snap::DEFAULT_TOLERANCE).delta
        } else {
            pos
        };
        self.pending_annotation = None;
        self.pending_effect = None;
        // **不立刻清掉旧选区**：清掉会让"选区内双击确认"失效（第二次点击时
        // 已经没有 inside_selection 了）。真正的替换发生在指针移动之后——
        // `update_selection_drag` 会用新矩形覆盖它。于是：
        //   * 只是点了一下 → 旧选区留着（若鼠标下有高亮窗口则改选那个窗口）；
        //   * 真的拖了 → 变成新框选的矩形。
        self.active_snap = SnapOutcome::default();
        self.drag = Drag::Create { anchor };
    }

    fn update_selection_drag(&mut self, pos: Pos2, screen: ERect) {
        let snapping = self.snap_allowed();
        if !snapping {
            self.active_snap = SnapOutcome::default();
        }

        match self.drag.clone() {
            Drag::Create { anchor } => {
                let (pos, outcome) = if snapping {
                    let outcome = snap::snap_corner(pos, &self.snap_targets, snap::DEFAULT_TOLERANCE);
                    (pos + outcome.delta, outcome)
                } else {
                    (pos, SnapOutcome::default())
                };
                self.active_snap = outcome;
                self.selection = Some(clamp_rect(from_egui(ERect::from_two_pos(anchor, pos)), screen));
            }
            Drag::Move { grab, original } => {
                let mut moved = original.translate(pos - grab);
                if snapping {
                    let outcome = snap::snap_move(moved, &self.snap_targets, snap::DEFAULT_TOLERANCE);
                    moved = moved.translate(outcome.delta);
                    self.active_snap = outcome;
                }
                // 夹回屏幕之后再决定最终矩形：夹取会改变边位置，
                // 但夹取后的结果必须仍然是一个尺寸合法的选区
                self.selection = Some(clamp_rect_move(from_egui(original), moved, screen));
            }
            Drag::Resize { handle, original } => {
                let mut rect = resize_rect(original, handle, pos, screen);
                if snapping {
                    let moving = MovingEdges {
                        left: handle.moves_left(),
                        right: handle.moves_right(),
                        top: handle.moves_top(),
                        bottom: handle.moves_bottom(),
                    };
                    let outcome =
                        snap::snap_resize(rect, moving, &self.snap_targets, snap::DEFAULT_TOLERANCE);
                    rect = rect.translate(outcome.delta);
                    self.active_snap = outcome;
                }
                self.selection = Some(clamp_rect(from_egui(rect), screen));
            }
            _ => {}
        }
    }

    fn update_annotation_drag(&mut self, local: Pos2) {
        let Drag::Annotate { anchor } = self.drag else {
            return;
        };
        match self.settings.tool {
            ToolKind::Pen | ToolKind::Highlighter => {
                // 轨迹抽稀：相邻点小于 1.5px 不记录，避免一次拖拽产生上万点
                let mut points = match self.pending_annotation.take() {
                    Some(Annotation::Freehand { points, .. })
                    | Some(Annotation::Highlighter { points, .. }) => points,
                    _ => Vec::new(),
                };
                if points.last().is_none_or(|p| p.distance(local) >= 1.5) {
                    points.push(local);
                }
                self.pending_annotation =
                    Some(self.make_annotation(self.settings.tool, anchor, local, &points));
            }
            ToolKind::Mosaic | ToolKind::Blur => {
                self.pending_effect = Some(from_egui(ERect::from_two_pos(anchor, local)));
            }
            tool => {
                self.pending_annotation = Some(self.make_annotation(tool, anchor, local, &[]));
            }
        }
    }

    /// 鼠标底下那个可吸附窗口的矩形（**取面积最小的那个**）。
    ///
    /// 为什么要挑最小的：窗口是重叠的（最大化浏览器上压着一个小对话框），用户点的是
    /// **看得见的那一个**；取"枚举顺序里第一个匹配"会给出不可预期的结果。
    fn hovered_snap_rect(&mut self, pos: Pos2) -> Option<ERect> {
        if !self.snap_allowed() {
            return None;
        }
        let window = smallest_containing(&self.snap_rects, pos);
        // 控件级吸附：问 UI Automation 要"这个点下面最深那个控件"。
        // 它比窗口级更精确（能吸到按钮/输入框），但**一次调用 5~8 ms**
        // （跨进程 COM，实测见 `capture::uia::control_at` 的日志），所以：
        //   * 只在**悬停**时问，拖拽中不问（拖拽要保持跟手）；
        //   * 按位置与时间节流（`control_probe`）。
        let Some(control) = self.control_snap_rect(pos) else {
            return window;
        };
        Some(control)
    }

    /// 节流地问一次"控件矩形"（拿不到就沿用上一次的答案，超时则当作没有）。
    ///
    /// 三个开关都关着时直接返回 `None`：`capture.snap_controls` 没开、按着 Alt、
    /// 或者整体吸附被关掉。
    fn control_snap_rect(&mut self, pos: Pos2) -> Option<ERect> {
        if !self.control_snap_enabled || !self.snap_allowed() {
            return None;
        }
        let now = std::time::Instant::now();
        let moved = self
            .control_probe
            .as_ref()
            .is_none_or(|probe| (pos - probe.at).length() > CONTROL_PROBE_TOLERANCE);
        let stale = self
            .control_probe
            .as_ref()
            .is_none_or(|probe| now.duration_since(probe.queried_at) > CONTROL_PROBE_INTERVAL);
        match &self.control_probe {
            // 位置没怎么动、答案也还新鲜：直接用上次结果
            Some(probe) if !moved || !stale => return probe.rect,
            _ => {}
        }

        // 遮罩局部坐标 → 全局物理像素（遮罩的 ppp 固定为 1.0，两者只差一个平移）
        let global = cuter_core::geom::Point::new(
            self.bounds.x + pos.x.round() as i32,
            self.bounds.y + pos.y.round() as i32,
        );
        let rect = cuter_core::capture::uia::control_at(global).map(|rect| {
            to_egui(rect.translate(-self.bounds.x, -self.bounds.y))
        });
        self.control_probe = Some(ControlProbe {
            at: pos,
            queried_at: now,
            rect,
        });
        rect
    }

    /// 松手时判断"这是一次点击而不是拖拽"。
    ///
    /// 单击高亮的窗口 = 直接把它作为选区（主流截图工具的通用手势）：
    /// 用户已经看到窗口被高亮了，点一下却什么都没发生是说不通的。
    fn click_selects_hovered_window(&mut self, pos: Pos2, screen: ERect) {
        let Some(press) = self.press_pos else {
            return;
        };
        let Some(selection) = click_target_selection(press, pos, self.click_target, screen) else {
            return;
        };
        log::info!("单击选窗口：选中 {:?}", selection);
        self.selection = Some(selection);
        self.notify(fill!(tr!("{} × {} 像素（单击选中窗口）"), selection.w, selection.h));
    }

    fn finish_drag(&mut self) {
        // 任何拖拽结束都要清掉吸附参考线，否则松手后画面里会留着一条线
        self.active_snap = SnapOutcome::default();
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None => {}
            Drag::Create { .. } | Drag::Move { .. } | Drag::Resize { .. } => {
                if let Some(selection) = self.selection
                    && (selection.w < MIN_SELECTION || selection.h < MIN_SELECTION)
                {
                    // 误触：太小的框选直接丢弃
                    self.selection = None;
                }
                let selection = self.selection;
                let text = match selection {
                    Some(s) => fill!(tr!("{} × {} 像素"), s.w, s.h),
                    None => tr!("拖拽以框选区域").to_owned(),
                };
                self.notify(text);
            }
            Drag::Annotate { .. } => {
                if let Some(rect) = self.pending_effect.take() {
                    match self.settings.tool {
                        ToolKind::Mosaic => self.doc.apply_mosaic(rect, self.settings.mosaic_block),
                        ToolKind::Blur => {
                            self.doc.apply_blur(rect, self.settings.blur_radius, 2);
                        }
                        _ => {}
                    }
                }
                if let Some(annotation) = self.pending_annotation.take()
                    && !is_degenerate(&annotation)
                {
                    self.doc.push_annotation(annotation);
                }
            }
        }
    }

    /// 构造一个标注：绘制预览与最终提交共用同一函数，保证「所见即所得」。
    fn make_annotation(
        &self,
        tool: ToolKind,
        anchor: Pos2,
        current: Pos2,
        points: &[Pos2],
    ) -> Annotation {
        let stroke = self.settings.stroke();
        match tool {
            ToolKind::Rectangle => Annotation::Rect {
                rect: ERect::from_two_pos(anchor, current),
                stroke,
                fill: self.settings.fill_color(),
                rounding: 0.0,
            },
            ToolKind::Ellipse => Annotation::Ellipse {
                rect: ERect::from_two_pos(anchor, current),
                stroke,
                fill: self.settings.fill_color(),
            },
            ToolKind::Arrow => Annotation::Arrow {
                from: anchor,
                to: current,
                stroke,
                head_len: (self.settings.stroke_width * 4.0).max(10.0),
            },
            ToolKind::Pen => Annotation::Freehand {
                points: points.to_vec(),
                stroke,
            },
            ToolKind::Highlighter => Annotation::Highlighter {
                points: points.to_vec(),
                color: self.settings.highlighter_color(),
                width: self.settings.stroke_width * 3.0,
            },
            // 马赛克/模糊是破坏性像素效果，不产生矢量标注；预览用细边框
            ToolKind::Mosaic | ToolKind::Blur => Annotation::Rect {
                rect: ERect::from_two_pos(anchor, current),
                stroke: Stroke::new(1.0, self.settings.color),
                fill: None,
                rounding: 0.0,
            },
            ToolKind::Select | ToolKind::Text | ToolKind::Counter | ToolKind::Laser => {
                Annotation::Rect {
                    rect: ERect::from_two_pos(anchor, current),
                    stroke,
                    fill: None,
                    rounding: 0.0,
                }
            }
        }
    }

    /// 遮罩局部坐标 → 文档坐标（两者同系，集中在此便于将来支持「画布缩放」）。
    fn document_pos(&self, pos: Pos2) -> Pos2 {
        pos
    }

    /// 工具条的屏幕矩形（用于让出鼠标事件与光标形状）。
    fn toolbar_rect(&self, screen: ERect) -> Option<ERect> {
        let selection = self.selection?;
        let size = toolbar::desired_size();
        let pos = toolbar::position(to_egui(selection), size, screen);
        Some(ERect::from_min_size(pos, size))
    }

    fn update_cursor_icon(&self, ctx: &egui::Context, screen: ERect) {
        let Some(pos) = self.cursor else {
            return;
        };
        if self.toolbar_rect(screen).is_some_and(|r| r.contains(pos)) {
            ctx.set_cursor_icon(CursorIcon::Default);
            return;
        }
        match self.drag {
            Drag::Resize { handle, .. } => ctx.set_cursor_icon(handle.cursor()),
            Drag::Move { .. } => ctx.set_cursor_icon(CursorIcon::Grabbing),
            _ => {
                if let Some(selection) = self.selection {
                    let rect = to_egui(selection);
                    if let Some(handle) = handle_at_in_screen(rect, pos, screen) {
                        ctx.set_cursor_icon(handle.cursor());
                        return;
                    }
                    if rect.contains(pos) && self.settings.tool == ToolKind::Select {
                        ctx.set_cursor_icon(CursorIcon::Grab);
                        return;
                    }
                }
                ctx.set_cursor_icon(match self.settings.tool {
                    ToolKind::Text => CursorIcon::Text,
                    ToolKind::Select => CursorIcon::Crosshair,
                    _ => CursorIcon::Crosshair,
                });
            }
        }
    }

    fn reset_selection(&mut self) {
        self.selection = None;
        self.pending_annotation = None;
        self.pending_effect = None;
        self.text_input = None;
        self.active_snap = SnapOutcome::default();
        self.drag = Drag::None;
    }

    fn result(&self, action: OverlayAction) -> OverlayResult {
        let region = self.selection_global().unwrap_or(Rect::ZERO);
        let origin = if region.is_empty() {
            self.bounds.origin()
        } else {
            region.origin()
        };
        OverlayResult {
            action,
            frame: match action {
                OverlayAction::Cancel
                | OverlayAction::Recapture
                | OverlayAction::OpenSettings => None,
                _ => self.render(),
            },
            origin,
            region,
        }
    }

    fn apply_toolbar(&mut self, action: ToolbarAction) -> Option<OverlayResult> {
        match action {
            ToolbarAction::SetTool(tool) => {
                self.toolbar.settings.tool = tool;
                self.settings.tool = tool;
                None
            }
            ToolbarAction::SetColor(color) => {
                self.toolbar.settings.color = color;
                self.settings.color = color;
                None
            }
            ToolbarAction::SetWidth(width) => {
                self.toolbar.settings.stroke_width = width;
                self.settings.stroke_width = width;
                None
            }
            ToolbarAction::SetFill(fill) => {
                self.toolbar.settings.fill_shapes = fill;
                self.settings.fill_shapes = fill;
                None
            }
            ToolbarAction::SetStyle(settings) => {
                // 弹窗里调好的样式：遮罩内立即生效，并挂到"待上报"槽里。
                //
                // **不能走 `OverlayResult`**：宿主收到任何结果都会销毁遮罩，
                // 那样"拖一下线宽滑块把遮罩关了"——所以这里走旁路，
                // 由宿主每帧 `take_style_change()` 取走写进配置。
                self.toolbar.settings = *settings.clone();
                self.settings = *settings;
                self.pending_style = Some(self.settings.clone());
                None
            }
            ToolbarAction::SetOpacity(opacity) => {
                self.toolbar.opacity = opacity;
                None
            }
            ToolbarAction::Undo => {
                let ok = self.doc.undo();
                self.notify(if ok { tr!("已撤销") } else { tr!("没有可撤销的操作") });
                None
            }
            ToolbarAction::Redo => {
                let ok = self.doc.redo();
                self.notify(if ok { tr!("已重做") } else { tr!("没有可重做的操作") });
                None
            }
            ToolbarAction::Clear => {
                self.doc.clear_annotations();
                self.notify(tr!("已清空标注"));
                None
            }
            ToolbarAction::ToggleMagnifier => {
                self.magnifier = !self.magnifier;
                self.toolbar.magnifier = self.magnifier;
                None
            }
            ToolbarAction::ToggleSnap => {
                self.snap_enabled = !self.snap_enabled;
                self.toolbar.snap_enabled = self.snap_enabled;
                self.notify(if self.snap_enabled {
                    tr!("窗口吸附：开")
                } else {
                    tr!("窗口吸附：关（按住 Alt 可临时关闭）")
                });
                None
            }
            ToolbarAction::Copy => Some(self.result(OverlayAction::Copy)),
            ToolbarAction::Save => Some(self.result(OverlayAction::Save)),
            ToolbarAction::SaveAs => Some(self.result(OverlayAction::SaveAs)),
            ToolbarAction::Pin => Some(self.result(OverlayAction::Pin)),
            ToolbarAction::Ocr => Some(self.result(OverlayAction::Ocr)),
            ToolbarAction::Scroll => Some(self.result(OverlayAction::ScrollCapture)),
            ToolbarAction::OpenSettings => Some(self.result(OverlayAction::OpenSettings)),
            ToolbarAction::Recapture => Some(self.result(OverlayAction::Recapture)),
            ToolbarAction::Close => Some(self.result(OverlayAction::Cancel)),
        }
    }

    // ------------------------------------------------------------------
    // 文字输入
    // ------------------------------------------------------------------

    fn text_input_ui(&mut self, ctx: &egui::Context) -> Option<OverlayResult> {
        let mut input = self.text_input.take()?;
        let mut commit = false;
        let mut cancel = false;
        let size = self.settings.font_size;
        let color = self.settings.color;

        egui::Area::new(Id::new("cuter-text-input"))
            .order(Order::Foreground)
            .fixed_pos(input.pos)
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut input.buffer)
                        .desired_width(200.0)
                        .font(egui::FontId::proportional(size))
                        .text_color(color),
                );
                if input.fresh {
                    response.request_focus();
                    input.fresh = false;
                }
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    commit = true;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
                if response.lost_focus() {
                    commit = true;
                }
            });

        if cancel {
            self.notify(tr!("已取消文字"));
            return None;
        }
        if commit {
            if !input.buffer.trim().is_empty() {
                self.doc
                    .push_text(input.pos, input.buffer.clone(), color, size);
            }
            self.set_tool(ToolKind::Select);
            return None;
        }
        self.text_input = Some(input);
        None
    }

    fn set_tool(&mut self, tool: ToolKind) {
        self.settings.tool = tool;
        self.toolbar.settings.tool = tool;
    }

    // ------------------------------------------------------------------
    // 绘制
    // ------------------------------------------------------------------

    fn paint(&mut self, ctx: &egui::Context, screen: ERect) {
        let background = ctx.layer_painter(LayerId::new(Order::Background, Id::new("cuter-shot")));
        let chrome = ctx.layer_painter(LayerId::new(Order::Middle, Id::new("cuter-chrome")));
        let size = self.doc.size();
        let full_uv = ERect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let texture = self.doc.texture(ctx).id();

        // 1) 整屏：压暗的冻结帧
        background.image(texture, screen, full_uv, DIM_TINT);

        // 1b) 还没开始框选时，高亮鼠标所在的窗口——这是"这里能吸附"的预告，
        //     用户不用先拖一下才知道有没有对准。
        if self.selection.is_none()
            && self.drag == Drag::None
            && self.snap_allowed()
            && let Some(cursor) = self.cursor
            && let Some(rect) = self.hovered_snap_rect(cursor)
        {
            chrome.rect_filled(rect, egui::CornerRadius::ZERO, self.theme.hover_tint);
            chrome.rect_stroke(
                rect,
                egui::CornerRadius::ZERO,
                Stroke::new(1.0, self.theme.accent),
                StrokeKind::Inside,
            );
        }

        // 2) 选区：原始亮度 + 标注
        if let Some(selection) = self.selection {
            let rect = to_egui(selection);
            if rect.width() > 0.0 && rect.height() > 0.0 {
                background.image(texture, rect, uv_of(selection, size), Color32::WHITE);

                let clipped = chrome.with_clip_rect(rect);
                self.doc.paint_annotations(&clipped, Vec2::ZERO);
                if let Some(pending) = &self.pending_annotation {
                    pending.paint(&clipped, Vec2::ZERO);
                }
                if let Some(effect) = self.pending_effect {
                    clipped.rect_stroke(
                        to_egui(effect),
                        egui::CornerRadius::ZERO,
                        Stroke::new(1.0, self.theme.accent),
                        StrokeKind::Outside,
                    );
                }

                chrome.rect_stroke(
                    rect,
                    egui::CornerRadius::ZERO,
                    Stroke::new(1.0, self.theme.accent),
                    StrokeKind::Outside,
                );
                for handle in Handle::ALL {
                    let square =
                        ERect::from_center_size(handle.position(rect), Vec2::splat(metrics::HANDLE_RADIUS * 2.0));
                    chrome.rect_filled(square, egui::CornerRadius::same(1), self.theme.handle_fill);
                    chrome.rect_stroke(
                        square,
                        egui::CornerRadius::same(1),
                        Stroke::new(1.0, self.theme.handle_stroke),
                        StrokeKind::Outside,
                    );
                }

                self.paint_size_chip(&chrome, rect, screen);
            }
        }

        // 3) 十字辅助线与放大镜
        if let Some(cursor) = self.cursor
            && !self.toolbar_rect(screen).is_some_and(|r| r.contains(cursor))
        {
            let guides = self.selection.is_none()
                || matches!(self.drag, Drag::Create { .. } | Drag::Resize { .. })
                || (self.drag == Drag::None && self.settings.tool != ToolKind::Select);
            if guides {
                chrome.line_segment(
                    [
                        Pos2::new(screen.left(), cursor.y),
                        Pos2::new(screen.right(), cursor.y),
                    ],
                    Stroke::new(1.0, self.theme.crosshair),
                );
                chrome.line_segment(
                    [
                        Pos2::new(cursor.x, screen.top()),
                        Pos2::new(cursor.x, screen.bottom()),
                    ],
                    Stroke::new(1.0, self.theme.crosshair),
                );
            }
            if self.magnifier {
                self.paint_magnifier(&chrome, screen, cursor, texture);
            }
        }

        // 4) 吸附参考线：横竖各一条贯穿整屏的淡青线。
        //    画在最上层，用户在拖拽时能立刻明白"吸到哪条边上了"。
        if self.active_snap.is_snapped()
            && matches!(
                self.drag,
                Drag::Create { .. } | Drag::Move { .. } | Drag::Resize { .. }
            )
        {
            let stroke = Stroke::new(1.0, self.theme.snap_line);
            if let Some(x) = self.active_snap.line_x {
                chrome.line_segment(
                    [Pos2::new(x, screen.top()), Pos2::new(x, screen.bottom())],
                    stroke,
                );
            }
            if let Some(y) = self.active_snap.line_y {
                chrome.line_segment(
                    [Pos2::new(screen.left(), y), Pos2::new(screen.right(), y)],
                    stroke,
                );
            }
        }
    }

    fn paint_size_chip(&self, painter: &Painter, rect: ERect, screen: ERect) {
        let Some(selection) = self.selection else {
            return;
        };
        // 22 → 27：给左上角控制点留出空隙，别被 pill 压住
        let above = rect.top() - 27.0;
        let anchor = if above > screen.top() {
            Pos2::new(rect.left(), above)
        } else {
            Pos2::new(rect.left() + 2.0, rect.top() + 4.0)
        };
        // 设计稿：尺寸与坐标合成一个 pill（「840 × 600　(130, 170)」）
        let text = format!(
            "{} × {}\u{3000}({}, {})",
            selection.w, selection.h, selection.x, selection.y
        );
        chip(painter, anchor, Align2::LEFT_TOP, &text, &self.theme);
    }

    fn paint_magnifier(
        &self,
        painter: &Painter,
        screen: ERect,
        cursor: Pos2,
        texture: egui::TextureId,
    ) {
        let box_size = metrics::MAGNIFIER;
        let source = box_size / metrics::MAGNIFIER_ZOOM;
        let size = self.doc.size();
        if size.is_empty() {
            return;
        }

        // 位置：默认光标右下，贴近边缘时翻到另一侧
        let mut origin = cursor + Vec2::new(24.0, 24.0);
        if origin.x + box_size > screen.right() {
            origin.x = cursor.x - 24.0 - box_size;
        }
        if origin.y + box_size > screen.bottom() {
            origin.y = cursor.y - 24.0 - box_size;
        }
        origin.x = origin
            .x
            .clamp(screen.left(), (screen.right() - box_size).max(screen.left()));
        origin.y = origin
            .y
            .clamp(screen.top(), (screen.bottom() - box_size).max(screen.top()));

        let dest = ERect::from_min_size(origin, Vec2::splat(box_size));
        let src = ERect::from_center_size(cursor, Vec2::splat(source));
        let uv = ERect::from_min_max(
            Pos2::new(src.left() / size.w as f32, src.top() / size.h as f32),
            Pos2::new(src.right() / size.w as f32, src.bottom() / size.h as f32),
        );

        painter.rect_filled(
            dest.expand(1.0),
            egui::CornerRadius::same(5),
            self.theme.label_bg,
        );
        painter.image(texture, dest, uv, Color32::WHITE);

        // 当前像素：放大后一格就是一个像素
        let center = dest.center();
        let cell = box_size / source;
        painter.rect_stroke(
            ERect::from_center_size(center, Vec2::splat(cell)),
            egui::CornerRadius::ZERO,
            Stroke::new(1.0, self.theme.accent),
            StrokeKind::Inside,
        );
        painter.line_segment(
            [
                Pos2::new(dest.left(), center.y),
                Pos2::new(dest.right(), center.y),
            ],
            Stroke::new(1.0, self.theme.crosshair),
        );
        painter.line_segment(
            [
                Pos2::new(center.x, dest.top()),
                Pos2::new(center.x, dest.bottom()),
            ],
            Stroke::new(1.0, self.theme.crosshair),
        );
        painter.rect_stroke(
            dest,
            egui::CornerRadius::same(4),
            Stroke::new(1.0, self.theme.magnifier_border),
            StrokeKind::Outside,
        );

        // 读数：颜色 + 全局像素坐标（见 `paint_readout`）
        let px = self
            .doc
            .base()
            .pixel(cursor.x.floor() as i32, cursor.y.floor() as i32);
        let global = self.bounds.origin() + Point::new(cursor.x.floor() as i32, cursor.y.floor() as i32);
        self.paint_readout(painter, dest, screen, px, global);
    }

    /// 放大镜底下的读数卡片。
    ///
    /// 早先是"色块 + 两个小胶囊"，三者宽度各不相同，贴在一起像三块碎片；改成一张与放大镜框
    /// 等宽的卡片：色块 + `#RRGGBB`（label 级、主色），下面两行说明级的 `rgb(...)` 与全局坐标。
    fn paint_readout(
        &self,
        painter: &Painter,
        anchor: ERect,
        screen: ERect,
        px: [u8; 4],
        global: Point,
    ) {
        let theme = &self.theme;
        let card = readout_rect(anchor, screen);
        let corner = egui::CornerRadius::same(metrics::CORNER_PANEL);
        painter.rect_filled(card, corner, theme.surface_raised);
        painter.rect_stroke(
            card,
            corner,
            Stroke::new(1.0, theme.outline),
            StrokeKind::Inside,
        );

        let pad = metrics::SPACE_2;
        let swatch = ERect::from_min_size(
            Pos2::new(card.left() + pad, card.top() + pad),
            Vec2::splat(22.0),
        );
        painter.rect_filled(
            swatch,
            egui::CornerRadius::same(metrics::CORNER_INPUT),
            Color32::from_rgb(px[0], px[1], px[2]),
        );
        painter.rect_stroke(
            swatch,
            egui::CornerRadius::same(metrics::CORNER_INPUT),
            Stroke::new(1.0, theme.outline_strong),
            StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(swatch.right() + pad, swatch.center().y),
            Align2::LEFT_CENTER,
            format!("#{:02X}{:02X}{:02X}", px[0], px[1], px[2]),
            egui::FontId::proportional(metrics::TEXT_LABEL),
            theme.on_surface,
        );
        painter.text(
            Pos2::new(card.left() + pad, swatch.bottom() + 6.0),
            Align2::LEFT_TOP,
            format!("rgb({}, {}, {})", px[0], px[1], px[2]),
            egui::FontId::proportional(metrics::TEXT_CAPTION),
            theme.on_surface_dim,
        );
        painter.text(
            Pos2::new(card.left() + pad, swatch.bottom() + 6.0 + 16.0),
            Align2::LEFT_TOP,
            format!("({}, {})", global.x, global.y),
            egui::FontId::proportional(metrics::TEXT_CAPTION),
            theme.on_surface_faint,
        );
    }
}

/// `capture_cfg` 目前只用于把「提示音」等偏好透传给宿主。
impl Overlay {
    #[must_use]
    pub fn capture_config(&self) -> &CaptureConfig {
        &self.capture_cfg
    }
}

// ---------------------------------------------------------------------------
// 纯函数工具（可无头测试）
// ---------------------------------------------------------------------------

fn to_egui(rect: Rect) -> ERect {
    ERect::from_min_size(
        Pos2::new(rect.x as f32, rect.y as f32),
        Vec2::new(rect.w as f32, rect.h as f32),
    )
}

fn from_egui(rect: ERect) -> Rect {
    Rect::from_corners(
        Point::new(rect.min.x.round() as i32, rect.min.y.round() as i32),
        Point::new(rect.max.x.round() as i32, rect.max.y.round() as i32),
    )
}

fn uv_of(rect: Rect, size: Size) -> ERect {
    let w = size.w.max(1) as f32;
    let h = size.h.max(1) as f32;
    ERect::from_min_max(
        Pos2::new(rect.x as f32 / w, rect.y as f32 / h),
        Pos2::new(rect.right() as f32 / w, rect.bottom() as f32 / h),
    )
}

fn clamp_pos(pos: Pos2, screen: ERect) -> Pos2 {
    Pos2::new(
        pos.x.clamp(screen.left(), screen.right()),
        pos.y.clamp(screen.top(), screen.bottom()),
    )
}

fn screen_rect(screen: ERect) -> Rect {
    Rect::new(
        screen.min.x.round() as i32,
        screen.min.y.round() as i32,
        screen.width().round().max(1.0) as u32,
        screen.height().round().max(1.0) as u32,
    )
}

fn clamp_rect(rect: Rect, screen: ERect) -> Rect {
    rect.intersect(screen_rect(screen))
}

/// 平移时保持尺寸不变，只是把整体夹回屏幕内。
fn clamp_rect_move(original: Rect, moved: ERect, screen: ERect) -> Rect {
    let mut rect = from_egui(moved);
    rect.w = original.w;
    rect.h = original.h;
    let bounds = screen_rect(screen);
    let max_x = (bounds.right() - rect.w as i32).max(bounds.left());
    let max_y = (bounds.bottom() - rect.h as i32).max(bounds.top());
    rect.x = rect.x.clamp(bounds.left(), max_x);
    rect.y = rect.y.clamp(bounds.top(), max_y);
    rect
}

fn resize_rect(original: ERect, handle: Handle, pos: Pos2, screen: ERect) -> ERect {
    let mut min = original.min;
    let mut max = original.max;
    if handle.moves_left() {
        min.x = pos.x;
    }
    if handle.moves_right() {
        max.x = pos.x;
    }
    if handle.moves_top() {
        min.y = pos.y;
    }
    if handle.moves_bottom() {
        max.y = pos.y;
    }
    ERect::from_two_pos(min, max).intersect(screen)
}

/// 命中测试用的控制点：**贴屏幕边的那几个不算**。
///
/// 选区贴边时（整屏截图、`Ctrl+R` 复原的上次区域、吸附到窗口边），它的控制点正好落在
/// 屏幕边界上——而用户把指针移到屏幕边缘，本意往往是"从边上再重新框一块"。原来的顺序是
/// 控制点优先，于是光标先变成缩放双箭头，一拖就把整块选区连图带着缩，看起来像
/// "屏幕帧被拖走了"（用户反馈，见 HANDOFF 第 84 节）。
///
/// 贴着屏幕边的那些边本来就无法再往外扩，跳过它们不影响正常缩放；
/// 想缩小仍然可以从对面那条边拖。
fn handle_at_in_screen(rect: ERect, pos: Pos2, screen: ERect) -> Option<Handle> {
    Handle::ALL
        .into_iter()
        .filter(|h| {
            let p = h.position(rect);
            p.x > screen.min.x + 1.0
                && p.x < screen.max.x - 1.0
                && p.y > screen.min.y + 1.0
                && p.y < screen.max.y - 1.0
        })
        .find(|h| h.position(rect).distance(pos) <= metrics::HANDLE_HIT)
}

/// 退化标注（面积/长度过小）不入库，避免误点产生看不见的对象。
fn is_degenerate(annotation: &Annotation) -> bool {
    match annotation {
        Annotation::Rect { rect, .. } | Annotation::Ellipse { rect, .. } => {
            rect.width() < 2.0 || rect.height() < 2.0
        }
        Annotation::Arrow { from, to, .. } => from.distance(*to) < 3.0,
        Annotation::Freehand { points, .. } | Annotation::Highlighter { points, .. } => {
            points.len() < 2
        }
        Annotation::Text { text, .. } => text.trim().is_empty(),
        Annotation::Counter { .. } => false,
    }
}

/// 画一个「胶囊」背景 + 文本的小标签，返回其矩形。
fn chip(painter: &Painter, anchor: Pos2, align: Align2, text: &str, theme: &Theme) -> ERect {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        theme.label_fg,
    );
    let padding = Vec2::new(7.0, 4.0);
    let rect = align.anchor_size(anchor, galley.size() + padding * 2.0);
    painter.rect_filled(rect, egui::CornerRadius::same(6), theme.label_bg);
    painter.galley(rect.min + padding, galley, theme.label_fg);
    rect
}

/// 放大镜读数卡片的高度与到放大镜框的间距。
const READOUT_HEIGHT: f32 = 76.0;
const READOUT_GAP: f32 = 6.0;

/// 放大镜读数卡片的位置：与放大镜框等宽，默认挂在框下 [`READOUT_GAP`] 点。
///
/// 屏幕下方放不下就翻到框上面；两个方向都越界时按屏幕边缘夹住。纯几何，方便直接测。
fn readout_rect(anchor: ERect, screen: ERect) -> ERect {
    let size = Vec2::new(anchor.width().max(metrics::MAGNIFIER), READOUT_HEIGHT);
    let mut origin = Pos2::new(anchor.left(), anchor.bottom() + READOUT_GAP);
    if origin.y + size.y > screen.bottom() {
        origin.y = anchor.top() - READOUT_GAP - size.y;
    }
    origin.x = origin.x.clamp(
        screen.left(),
        (screen.right() - size.x).max(screen.left()),
    );
    origin.y = origin.y.clamp(
        screen.top(),
        (screen.bottom() - size.y).max(screen.top()),
    );
    ERect::from_min_size(origin, size)
}

/// 从候选矩形里挑出包含 `pos` 且**面积最小**的那个。
fn smallest_containing(rects: &[ERect], pos: Pos2) -> Option<ERect> {
    rects
        .iter()
        .filter(|rect| rect.contains(pos))
        .min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
        .copied()
}

/// 「单击选中高亮窗口」的判定：纯函数，方便直接测。
///
/// 返回 `None` 表示这次不是"单击选窗口"：移动超过 [`CLICK_SLOP`]（是拖拽）、
/// 按下时没有高亮窗口（调用方已经按工具/选区状态过滤过）、或者夹取之后选区太小。
fn click_target_selection(
    press: Pos2,
    release: Pos2,
    target: Option<ERect>,
    screen: ERect,
) -> Option<Rect> {
    if release.distance(press) > CLICK_SLOP {
        return None;
    }
    let clamped = clamp_rect(from_egui(target?), screen);
    if clamped.w < MIN_SELECTION || clamped.h < MIN_SELECTION {
        return None;
    }
    Some(clamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> ERect {
        ERect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0))
    }

    #[test]
    fn the_readout_card_hangs_below_the_magnifier() {
        let anchor = ERect::from_min_size(Pos2::new(300.0, 200.0), Vec2::new(140.0, 140.0));
        let card = readout_rect(anchor, screen());
        // 与放大镜框左对齐、等宽，挂在框下面
        assert_eq!(card.left(), anchor.left());
        assert_eq!(card.width(), anchor.width());
        assert_eq!(card.top(), anchor.bottom() + READOUT_GAP);
    }

    #[test]
    fn the_readout_card_flips_up_near_the_bottom_edge() {
        let anchor = ERect::from_min_size(Pos2::new(300.0, 700.0), Vec2::new(140.0, 140.0));
        let card = readout_rect(anchor, screen());
        // 框下面放不下 → 翻到框上面
        assert_eq!(card.bottom(), anchor.top() - READOUT_GAP);
        assert!(card.bottom() <= screen().bottom());
    }

    #[test]
    fn the_readout_card_stays_on_screen_in_the_corner() {
        let anchor = ERect::from_min_size(Pos2::new(980.0, 780.0), Vec2::new(140.0, 140.0));
        let card = readout_rect(anchor, screen());
        assert!(card.right() <= screen().right());
        assert!(card.bottom() <= screen().bottom());
        assert!(card.left() >= screen().left() && card.top() >= screen().top());
    }

    #[test]
    fn smallest_containing_prefers_the_inner_window() {
        let big = ERect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1000.0, 800.0));
        let small = ERect::from_min_size(Pos2::new(400.0, 300.0), Vec2::new(200.0, 150.0));
        let rects = vec![big, small];
        // 小窗口压在最大化窗口上面：鼠标在小窗口里必须选中小的那个
        assert_eq!(smallest_containing(&rects, Pos2::new(500.0, 400.0)), Some(small));
        // 小窗口之外仍然是大的
        assert_eq!(smallest_containing(&rects, Pos2::new(100.0, 100.0)), Some(big));
        // 谁都不包含 → 没有
        assert_eq!(smallest_containing(&rects, Pos2::new(1200.0, 100.0)), None);
    }

    #[test]
    fn click_on_a_highlighted_window_becomes_the_selection() {
        let target = ERect::from_min_size(Pos2::new(100.0, 120.0), Vec2::new(300.0, 200.0));
        // 单击（几乎没有移动）→ 用窗口矩形作选区
        let picked = click_target_selection(
            Pos2::new(200.0, 200.0),
            Pos2::new(201.0, 200.5),
            Some(target),
            screen(),
        );
        assert_eq!(picked, Some(Rect::new(100, 120, 300, 200)));

        // 拖拽（移动超过阈值）→ 不是"选窗口"，交给正常的框选
        assert_eq!(
            click_target_selection(
                Pos2::new(200.0, 200.0),
                Pos2::new(260.0, 240.0),
                Some(target),
                screen()
            ),
            None
        );

        // 按下时没有高亮窗口 → 什么也不做（正常拖出小选区）
        assert_eq!(
            click_target_selection(
                Pos2::new(200.0, 200.0),
                Pos2::new(200.0, 200.0),
                None,
                screen()
            ),
            None
        );
    }

    #[test]
    fn click_selection_is_clamped_to_the_screen() {
        // 窗口有一半在屏幕外（多屏/跨屏窗口）：夹到屏幕内，且不能退化成空选区
        let target = ERect::from_min_size(Pos2::new(-200.0, 700.0), Vec2::new(400.0, 300.0));
        let picked = click_target_selection(
            Pos2::new(10.0, 710.0),
            Pos2::new(10.0, 710.0),
            Some(target),
            screen(),
        )
        .expect("夹取之后仍然是合法选区");
        assert_eq!(picked, Rect::new(0, 700, 200, 100));
    }

    #[test]
    fn rect_egui_roundtrip_is_lossless_for_integers() {
        let r = Rect::new(-10, 20, 300, 400);
        assert_eq!(from_egui(to_egui(r)), r);
    }

    #[test]
    fn uv_maps_selection_into_texture_space() {
        let size = Size::new(1000, 500);
        let uv = uv_of(Rect::new(500, 250, 500, 250), size);
        assert!((uv.min.x - 0.5).abs() < 1e-6);
        assert!((uv.min.y - 0.5).abs() < 1e-6);
        assert!((uv.max.x - 1.0).abs() < 1e-6);
        assert!((uv.max.y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn handle_hit_test_finds_corners_and_misses_center() {
        let rect = ERect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        assert_eq!(handle_at_in_screen(rect, Pos2::new(100.0, 100.0), screen()), Some(Handle::NW));
        assert_eq!(handle_at_in_screen(rect, Pos2::new(300.0, 300.0), screen()), Some(Handle::SE));
        assert_eq!(handle_at_in_screen(rect, Pos2::new(200.0, 100.0), screen()), Some(Handle::N));
        assert_eq!(handle_at_in_screen(rect, Pos2::new(200.0, 200.0), screen()), None);
    }

    #[test]
    fn handles_that_sit_on_the_screen_border_are_ignored() {
        let screen = ERect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1920.0, 1080.0));
        // 整屏选区：四周的控制点全在屏幕边界上，一个都不该命中，
        // 这样指针贴到屏幕边时是"新框选"而不是"缩放整块选区"。
        for probe in [
            Pos2::new(0.0, 0.0),
            Pos2::new(1920.0, 1080.0),
            Pos2::new(1920.0, 540.0),
            Pos2::new(960.0, 0.0),
        ] {
            assert_eq!(handle_at_in_screen(screen, probe, screen), None);
        }
        // 离开屏幕边的选区，控制点照常可用。
        let inset = ERect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        assert_eq!(
            handle_at_in_screen(inset, Pos2::new(300.0, 300.0), screen),
            Some(Handle::SE)
        );
        // 贴右边界的那条边不再可拖，但左边还在。
        let right_flush = ERect::from_min_size(Pos2::new(1500.0, 100.0), Vec2::new(420.0, 300.0));
        assert_eq!(
            handle_at_in_screen(right_flush, Pos2::new(1920.0, 250.0), screen),
            None
        );
        assert_eq!(
            handle_at_in_screen(right_flush, Pos2::new(1500.0, 250.0), screen),
            Some(Handle::W)
        );
    }

    #[test]
    fn resize_from_west_moves_only_the_left_edge() {
        let original = ERect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let result = resize_rect(original, Handle::W, Pos2::new(150.0, 500.0), screen());
        assert_eq!(result.min, Pos2::new(150.0, 100.0));
        assert_eq!(result.max, Pos2::new(300.0, 300.0));
    }

    #[test]
    fn resize_can_flip_the_rectangle_without_negative_size() {
        let original = ERect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let result = resize_rect(original, Handle::W, Pos2::new(400.0, 200.0), screen());
        assert!(result.width() > 0.0);
        assert_eq!(result.min.x, 300.0);
    }

    #[test]
    fn resize_is_clamped_to_the_screen() {
        let original = ERect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let result = resize_rect(original, Handle::E, Pos2::new(5000.0, 200.0), screen());
        assert!(result.max.x <= 1000.0);
    }

    #[test]
    fn create_rect_clamps_to_screen() {
        let rect = ERect::from_two_pos(Pos2::new(-50.0, -50.0), Pos2::new(120.0, 90.0));
        assert_eq!(from_egui(rect), Rect::new(-50, -50, 170, 140));
        assert_eq!(
            clamp_rect(from_egui(rect), screen()),
            Rect::new(0, 0, 120, 90)
        );
    }

    #[test]
    fn move_keeps_size_and_stays_inside() {
        let original = Rect::new(100, 100, 200, 150);
        let moved = to_egui(original).translate(Vec2::new(950.0, 0.0));
        let clamped = clamp_rect_move(original, moved, screen());
        assert_eq!((clamped.w, clamped.h), (200, 150));
        assert!(clamped.right() <= 1000);
    }

    #[test]
    fn move_from_negative_drag_lands_at_origin() {
        let original = Rect::new(100, 100, 200, 150);
        let moved = to_egui(original).translate(Vec2::new(-500.0, -500.0));
        let clamped = clamp_rect_move(original, moved, screen());
        assert_eq!((clamped.x, clamped.y), (0, 0));
        assert_eq!((clamped.w, clamped.h), (200, 150));
    }

    #[test]
    fn degenerate_annotations_are_rejected() {
        assert!(is_degenerate(&Annotation::Rect {
            rect: ERect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 1.0)),
            stroke: Stroke::new(1.0, Color32::RED),
            fill: None,
            rounding: 0.0,
        }));
        assert!(!is_degenerate(&Annotation::Rect {
            rect: ERect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
            stroke: Stroke::new(1.0, Color32::RED),
            fill: None,
            rounding: 0.0,
        }));
        assert!(is_degenerate(&Annotation::Freehand {
            points: vec![Pos2::ZERO],
            stroke: Stroke::new(1.0, Color32::RED),
        }));
        assert!(is_degenerate(&Annotation::Arrow {
            from: Pos2::ZERO,
            to: Pos2::new(1.0, 1.0),
            stroke: Stroke::new(1.0, Color32::RED),
            head_len: 4.0,
        }));
        assert!(!is_degenerate(&Annotation::Counter {
            center: Pos2::ZERO,
            index: 1,
            color: Color32::RED,
            radius: 10.0,
        }));
    }

    #[test]
    fn every_handle_moves_at_least_one_edge() {
        for handle in Handle::ALL {
            assert!(
                handle.moves_left()
                    || handle.moves_right()
                    || handle.moves_top()
                    || handle.moves_bottom(),
                "{handle:?} 不移动任何边"
            );
        }
    }

    #[test]
    fn clamp_pos_keeps_cursor_on_screen() {
        assert_eq!(
            clamp_pos(Pos2::new(-20.0, 5000.0), screen()),
            Pos2::new(0.0, 800.0)
        );
    }

    /// 造一个尺寸可控的遮罩，用于验证与像素无关的状态机。
    fn overlay(config: &Config) -> Overlay {
        use cuter_core::capture::DesktopShot;
        use cuter_core::frame::Frame;

        let shot = DesktopShot {
            frame: Frame::new(Size::new(400, 300)),
            bounds: Rect::new(0, 0, 400, 300),
            monitors: Vec::new(),
            cursor: None,
        };
        Overlay::new(shot, config, Theme::dark(), None, Vec::new())
    }

    #[test]
    fn enter_returns_the_scroll_action_only_in_scroll_mode() {
        let mut config = Config::default();
        config.ocr.auto_after_capture = false;
        config.save.auto_save = false;

        let mut o = overlay(&config);
        assert!(!o.scroll_mode(), "默认不是滚动模式");
        // 没有选区时按回车不应该产生任何动作
        assert!(o.confirm_default().action == OverlayAction::Copy);
        assert_eq!(o.confirm_default().frame, None, "没有选区就没有图");

        o.set_scroll_mode(true);
        assert!(o.scroll_mode());
        assert_eq!(o.confirm_default().action, OverlayAction::ScrollCapture);
    }

    #[test]
    fn scroll_mode_outranks_auto_ocr_and_auto_save() {
        let mut config = Config::default();
        config.ocr.auto_after_capture = true;
        config.save.auto_save = true;
        let mut o = overlay(&config);
        assert_eq!(o.confirm_default().action, OverlayAction::Ocr);
        // 显式进入滚动模式必须压过这两个自动行为，否则用户按了 Alt+S 却拿到了别的结果
        o.set_scroll_mode(true);
        assert_eq!(o.confirm_default().action, OverlayAction::ScrollCapture);
        o.set_scroll_mode(false);
        assert_eq!(o.confirm_default().action, OverlayAction::Ocr);
    }
}
