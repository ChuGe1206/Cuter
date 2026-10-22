//! 屏幕贴图窗口的内容与交互。
//!
//! 贴图 = 把一张图「钉」在屏幕上，可以缩放、拖动、随时复制或关闭。
//! 窗口本身由宿主（`cuter-app`）创建为**无边框 + 置顶 + 尺寸等于图片显示尺寸
//! （`pin.shadow` 打开时再加上一圈阴影留白）**。
//!
//! 交互约定（对齐主流截图工具，降低学习成本）：
//!
//! | 操作 | 行为 |
//! |------|------|
//! | 左键拖拽 | 移动窗口（交给系统的窗口拖拽） |
//! | 滚轮 | 缩放（`pin.wheel_zoom` 关闭时无效） |
//! | 中键 | 恢复 100% |
//! | 右键 | 弹出菜单（复制 / 保存 / 识别文字 / 不透明度 / 穿透 / 置顶 / 关闭） |
//! | Esc | 关闭 |
//! | 双击 | 关闭 |
//!
//! 三种模式的差别集中在**窗口尺寸归谁管**：
//!
//! * 常规：窗口尺寸 = 内容尺寸（+ 阴影留白），滚轮改缩放就下发一次
//!   [`PinAction::Resize`]；
//! * `pin.zoom_within_window`：窗口尺寸**固定不变**，只有内容在窗口内放大缩小，
//!   超出部分被窗口裁掉（同类工具里常见的做法）；
//! * `pin.shadow`：窗口比内容大一圈，多出来的空间用来画投影。

use crate::tr;
use crate::fill;

use egui::{
    Align2, Area, Color32, CornerRadius, Id, Key, Order, PointerButton, Pos2, Rect, Stroke,
    StrokeKind, Vec2, ViewportCommand,
};

use cuter_core::config::PinConfig;
use cuter_core::frame::Frame;
use cuter_core::geom::Size;

use crate::annotate::frame_to_color_image;
use crate::theme::Theme;

/// 贴图缩放的上下限（与 `PinConfig::clamp` 的 `initial_scale` 范围保持一致）。
const MIN_SCALE: f32 = 0.05;
/// 不透明度下限。菜单滑杆和 Ctrl+滚轮共用同一个下限，别让两条路走出两个区间。
const MIN_OPACITY: f32 = 0.1;
const MAX_SCALE: f32 = 16.0;

/// 阴影留白（逻辑像素）。打开 `pin.shadow` 时窗口四周各让出这么多，用来画投影。
///
/// 从 12 收到 7：12 的时候那圈在浅色桌面上读起来像**一圈灰边框**（用户反馈
/// "很宽的大边框"），而不是投影。层数同时加多、每层更淡，才是阴影的样子。
pub const SHADOW_MARGIN: f32 = 7.0;

/// 阴影分几层叠加（层数越多越柔和，代价只是多几笔矩形填充）。
const SHADOW_LAYERS: i32 = 9;

/// 每层叠加的不透明度（0~255）。九层累加后最内侧约 17%，最外侧只有 2%。
const SHADOW_ALPHA: u8 = 5;

/// 投影整体往下让一点，看起来像"浮在桌面上"而不是"描了一圈边"。
const SHADOW_DROP: f32 = 1.5;

/// Ctrl + 左键拖动多少像素才算「要拖出」（避免手一抖就起拖）。
const DRAG_OUT_THRESHOLD: f32 = 5.0;

/// 贴图窗口上的动作。
#[derive(Clone, Debug, PartialEq)]
pub enum PinAction {
    Close,
    Copy,
    Save,
    Ocr,
    /// 请求宿主把**窗口**尺寸调整为 `size`（egui 点，已含阴影留白）。
    ///
    /// `at` 供「从左边/上边拖拽缩放」使用：窗口一边变大一边要往左/往上挪，
    /// 抓在手里的那条边才跟得住指针。其余缩放路径传 `None`，位置不动。
    Resize {
        size: Size,
        at: Option<egui::Pos2>,
    },
    SetOpacity(f32),
    ToggleClickThrough(bool),
    /// 「始终置顶」被用户改动：宿主据此改窗口层级。
    SetAlwaysOnTop(bool),
    /// 请求宿主把这张图**拖到别的程序里**（Ctrl + 左键拖动）。
    ///
    /// 宿主负责落一个临时文件并起 OLE 拖放——那需要 HWND 与 COM，属于应用层的事。
    DragOut,
}

/// 贴图窗口的哪条边（或哪个角）被按住了。
///
/// 存在的理由：贴图窗口只有一个"整窗拖动"手感时，鼠标贴到图边是普通箭头，
/// 一拖就把整张图搬走。用户的原话是「鼠标放到屏幕边界就会变成鼠标箭头，
/// 这时候左键移动鼠标就会把屏幕帧给移动」——他们要的是**边可以拉**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PinEdge {
    N,
    S,
    E,
    W,
    Ne,
    Nw,
    Se,
    Sw,
}

impl PinEdge {
    fn cursor(self) -> egui::CursorIcon {
        match self {
            Self::E | Self::W => egui::CursorIcon::ResizeHorizontal,
            Self::N | Self::S => egui::CursorIcon::ResizeVertical,
            Self::Ne | Self::Sw => egui::CursorIcon::ResizeNeSw,
            Self::Nw | Self::Se => egui::CursorIcon::ResizeNwSe,
        }
    }

    /// 这条边是否响应水平方向的拖动。
    fn horizontal(self) -> bool {
        matches!(self, Self::E | Self::W | Self::Ne | Self::Nw | Self::Se | Self::Sw)
    }

    /// 这条边是否响应垂直方向的拖动。
    fn vertical(self) -> bool {
        matches!(self, Self::N | Self::S | Self::Ne | Self::Nw | Self::Se | Self::Sw)
    }

    /// 这条边在水平方向上应该往哪边算（+1 = 把手往右拖是放大）。
    fn x_sign(self) -> f32 {
        match self {
            Self::W | Self::Nw | Self::Sw => -1.0,
            _ => 1.0,
        }
    }

    /// 同上，垂直方向（+1 = 把手往下拖是放大）。
    fn y_sign(self) -> f32 {
        match self {
            Self::N | Self::Ne | Self::Nw => -1.0,
            _ => 1.0,
        }
    }
}

/// 贴图窗口边缘多宽的带状区域算"抓住了边"（egui 点）。
const EDGE_GRAB: f32 = 6.0;

/// 正在进行的边缘缩放。
#[derive(Clone, Copy, Debug)]
struct PinResize {
    edge: PinEdge,
    /// 按下时的指针位置（视口坐标）。
    start: egui::Pos2,
    /// 按下时的缩放。
    start_scale: f32,
    /// 按下时的内容尺寸（egui 点）。
    ///
    /// 必须记住**按下那一刻**的尺寸：拖动过程中窗口一直在变，若拿实时尺寸当分母，
    /// 越拖分母越小、同样的位移算出的相对变化越来越大，一次拖动会把贴图缩成一个点
    /// （实测 424×246 一把拖成 44×35）。
    start_content: egui::Vec2,
}

/// 指针落在哪条边/角上。`content` 是图片实际显示区域（不含阴影留白），
/// `window` 是整个窗口（图片 + 阴影留白）。
///
/// 先按"贴图片边"判断；不够的话再看是不是落在**阴影那一圈**里——用户看不见阴影
/// 的边界，把指针放到窗口最外层（看起来就是图片的外框）也该能拉边，否则贴图贴在
/// 屏幕边上时，最外面那几个像素永远只会移动窗口。
fn edge_at_window(content: egui::Rect, window: egui::Rect, pos: egui::Pos2) -> Option<PinEdge> {
    if let Some(edge) = edge_at(content, pos) {
        return Some(edge);
    }
    if !window.contains(pos) {
        return None;
    }
    let n = pos.y < content.top();
    let s = pos.y > content.bottom();
    let w = pos.x < content.left();
    let e = pos.x > content.right();
    match (n, s, w, e) {
        (true, _, true, _) => Some(PinEdge::Nw),
        (true, _, _, true) => Some(PinEdge::Ne),
        (_, true, true, _) => Some(PinEdge::Sw),
        (_, true, _, true) => Some(PinEdge::Se),
        (true, _, _, _) => Some(PinEdge::N),
        (_, true, _, _) => Some(PinEdge::S),
        (_, _, true, _) => Some(PinEdge::W),
        (_, _, _, true) => Some(PinEdge::E),
        _ => None,
    }
}

/// 指针落在哪条边/角上。`content` 是图片实际显示区域（不含阴影留白）。
fn edge_at(content: egui::Rect, pos: egui::Pos2) -> Option<PinEdge> {
    let outer = content.expand(EDGE_GRAB);
    if !outer.contains(pos) {
        return None;
    }
    let near_w = (pos.x - content.left()).abs() <= EDGE_GRAB;
    let near_e = (pos.x - content.right()).abs() <= EDGE_GRAB;
    let near_n = (pos.y - content.top()).abs() <= EDGE_GRAB;
    let near_s = (pos.y - content.bottom()).abs() <= EDGE_GRAB;
    // 角优先：在角上时两个方向都能拉，指到哪个角就给哪个角的箭头
    match (near_n, near_s, near_w, near_e) {
        (true, _, true, _) => Some(PinEdge::Nw),
        (true, _, _, true) => Some(PinEdge::Ne),
        (_, true, true, _) => Some(PinEdge::Sw),
        (_, true, _, true) => Some(PinEdge::Se),
        (true, _, _, _) => Some(PinEdge::N),
        (_, true, _, _) => Some(PinEdge::S),
        (_, _, true, _) => Some(PinEdge::W),
        (_, _, _, true) => Some(PinEdge::E),
        _ => None,
    }
}

/// 从按下到现在的拖动量，算出新的缩放。
///
/// 取"相对尺寸变化"而不是像素位移：同样是拖 40 点，小图变化大、大图变化小，
/// 这样手感在两种尺寸下一致。角上取变化更大的那个方向，避免斜着拖时反而变慢。
fn scale_after_drag(
    start_scale: f32,
    edge: PinEdge,
    delta: egui::Vec2,
    start_content: egui::Vec2,
) -> f32 {
    let w = start_content.x.max(1.0);
    let h = start_content.y.max(1.0);
    let rel_x = if edge.horizontal() {
        delta.x * edge.x_sign() / w
    } else {
        0.0
    };
    let rel_y = if edge.vertical() {
        delta.y * edge.y_sign() / h
    } else {
        0.0
    };
    let rel = if rel_x.abs() >= rel_y.abs() {
        rel_x
    } else {
        rel_y
    };
    (start_scale * (1.0 + rel)).clamp(MIN_SCALE, MAX_SCALE)
}

/// 右键菜单：宽 200、行高 32、圆角 12（设计系统「窗口与弹层」档），内边距 6。
/// 菜单「设计尺寸」（egui 点）。
///
/// 画在屏幕上时按 `ppp` 缩放，所以物理像素尺寸是 `MENU_WIDTH * ppp`——
/// 实测 ppp = 1.25 时菜单占 250×360 物理像素，与之相符。
const MENU_WIDTH: f32 = 200.0;
const MENU_HEIGHT: f32 = 288.0;
/// 菜单与窗口边缘之间留的余量（免得菜单贴着边界被描边切掉一像素）。
const MENU_PAD: f32 = 10.0;
const MENU_ROW_HEIGHT: f32 = 32.0;
const MENU_SEPARATOR_HEIGHT: f32 = 9.0;
const MENU_PAD_X: f32 = 10.0;
const MENU_PADDING: i8 = 6;
const MENU_CORNER: u8 = 12;
const SLIDER_WIDTH: f32 = 72.0;
const SLIDER_HINT_GAP: f32 = 34.0;
/// Ctrl + 滚轮每一格改多少不透明度（常见手感：一格格走，不走飞）。
const OPACITY_WHEEL_STEP: f32 = 0.05;
/// 一次性提示（滚轮调不透明度、开关穿透/置顶）显示多久（秒）。
const TOAST_SECONDS: f64 = 1.8;

/// 贴图状态。
pub struct Pin {
    frame: Frame,
    texture: Option<egui::TextureHandle>,
    /// 显示缩放（1.0 = 原始像素大小）。
    scale: f32,
    opacity: f32,
    click_through: bool,
    border: bool,
    /// 图片本身裁成圆角（阴影早就是圆角，图片还是直角会露出来）。
    rounded: bool,
    always_on_top: bool,
    /// 滚轮是否改变缩放（`pin.wheel_zoom`）。
    wheel_zoom: bool,
    /// 缩放时是否保持窗口尺寸不变（`pin.zoom_within_window`）。
    zoom_within_window: bool,
    /// 是否绘制投影（`pin.shadow`）。
    shadow: bool,
    /// 当前窗口的物理尺寸。
    window_size: Vec2,
    /// `zoom_within_window` 模式下窗口的固定尺寸（创建时确定，之后不再变）。
    base_window_size: Vec2,
    menu_open: bool,
    /// 为菜单额外撑出的窗口尺寸（与 `window_size` 同单位）；菜单关掉就归零。
    /// `content_rect` 会把它折回去，所以撑窗口**不会**拉伸贴图。
    menu_extra: Vec2,
    /// 撑大/挪动**之前**的窗口尺寸（物理像素），菜单开着期间用它当「窗口」来摆贴图。
    ///
    /// 不能拿 `window_size - menu_extra` 反推：挪窗口会让两者对不上，贴图就会被拉长。
    menu_window: Vec2,
    /// 撑大之前的窗口尺寸（物理像素），关掉菜单时还原
    menu_restore: Option<Vec2>,
    /// 为「菜单完整可见」把窗口挪动了多少（egui 点，往左上为负）。
    ///
    /// 挪窗口会让贴图在屏幕坐标里跟着跑，`content_rect` 按这个值反向补偿，
    /// 于是**贴图在屏幕上的位置一点没变**，多出来的空间全给菜单用；关掉菜单归零。
    menu_shift: Vec2,
    /// 已经下发过的 [`Self::menu_shift`]，用来判断本帧要不要再发一次 `OuterPosition`。
    menu_shift_sent: Vec2,
    /// 菜单打开那一刻，点击点在**还没挪过**的窗口里的坐标（egui 点，冻结值）。
    ///
    /// `grow_for_menu` 每帧都会跑，而 `at` 是相对当前窗口量的——窗口挪过之后同一个屏幕点
    /// 的视口坐标也会变。拿变过的 `at` 再算一遍摆放，就等于把位移重复应用，窗口会挪过头
    /// （实测挪了 2 倍，菜单只剩第一行）。所以开菜单时冻结一次，之后都用它。
    ///
    /// ⚠️ 单位是 **egui 点**，和传给 `grow_for_menu` 的 `at` 同一把尺子：这里存物理像素
    /// 的话，下游再乘一次 `pixels_per_point` 就翻倍（实测 `[105.6 56.8]` 点变
    /// `[217.6 312.0]`，菜单被推出窗口，只剩 4 行）。
    menu_at: Option<Vec2>,
    /// 菜单实际要画在窗口里的哪个点（视口坐标）。
    ///
    /// 挪窗口是为了让菜单落在显示器里，可菜单**本身**是贴着点击点画的；窗口一挪，
    /// 那处屏幕点在视口里的坐标就变了，得让它跟着平移，否则菜单会跑到窗口外被裁掉
    /// （实测：只挪窗口不挪菜单，菜单只剩 4 行）。菜单关掉后回到 `None`。
    menu_anchor: Option<Pos2>,
    /// 首次挪动前窗口的外框位置（egui 点），关掉菜单时挪回去。
    menu_pos_restore: Option<Pos2>,
    /// 按下拖出手势时的起点：超过几像素才算真拖，避免把误触当成拖出。
    drag_out_armed: Option<egui::Pos2>,
    /// 这次拖出用的是中键还是 Ctrl+左键（松手判断要用对按钮）。
    drag_out_middle: bool,
    /// 正从某条边/某个角拖拽缩放时记下的起点。
    ///
    /// 贴图窗口原先只有"整窗拖动"一种手感：鼠标贴到图边变成普通箭头，
    /// 一拖把整张图搬走（用户反馈"屏幕边界一拖帧就跑了"）。有了这个字段，
    /// 边/角上的拖动改走缩放，图的中间才是移动。
    resize: Option<PinResize>,

    theme: Theme,
    /// 一次性提示。
    status: Option<String>,
    /// 提示的到期时刻（`ctx.input(|i| i.time)`，秒）。`None` = 一直显示到下次被改。
    ///
    /// 分成两档是因为两类提示寿命不同：滚轮调不透明度、开关穿透是"回应刚才那一下"，
    /// 看过就该走；"已开启鼠标穿透"这类要一直留着（穿透状态下窗口收不到鼠标，
    /// 用户得知道怎么退出）。
    status_until: Option<f64>,
}

impl Pin {
    /// 创建贴图内容。`window_size` 为宿主实际创建的窗口尺寸（物理像素）。
    #[must_use]
    pub fn new(frame: Frame, config: &PinConfig, theme: Theme, window_size: Vec2) -> Self {
        Self {
            frame,
            texture: None,
            scale: config.initial_scale,
            opacity: config.opacity,
            click_through: config.click_through,
            border: config.border,
            rounded: config.rounded_corners,
            always_on_top: config.always_on_top,
            wheel_zoom: config.wheel_zoom,
            zoom_within_window: config.zoom_within_window,
            shadow: config.shadow,
            window_size,
            base_window_size: window_size,
            menu_open: false,
            menu_extra: Vec2::ZERO,
            menu_window: Vec2::ZERO,
            menu_restore: None,
            menu_shift: Vec2::ZERO,
            menu_shift_sent: Vec2::ZERO,
            menu_at: None,
            menu_anchor: None,
            menu_pos_restore: None,
            drag_out_armed: None,
            drag_out_middle: false,
            resize: None,
            theme,
            status: None,
            status_until: None,
        }
    }

    /// 图片原始尺寸。
    #[must_use]
    pub fn image_size(&self) -> Size {
        self.frame.size()
    }

    /// 当前缩放下的内容显示尺寸（**不含**阴影留白）。
    #[must_use]
    pub fn content_size(&self) -> Size {
        Size::new(
            ((self.frame.size().w as f32 * self.scale).round() as u32).max(1),
            ((self.frame.size().h as f32 * self.scale).round() as u32).max(1),
        )
    }

    /// 阴影留白（没开阴影时为 0）。
    #[must_use]
    pub fn margin(&self) -> f32 {
        if self.shadow { SHADOW_MARGIN } else { 0.0 }
    }

    /// 当前内容与设置下，窗口应该有多大。
    ///
    /// `zoom_within_window` 时窗口尺寸是钉死的（创建时的尺寸），这也是「只放大内容」
    /// 的字面含义；此时缩放**不会**产生 [`PinAction::Resize`]。
    ///
    /// 右键菜单开着时还要把菜单要的那块算进去（`menu_extra`，由
    /// [`Self::grow_for_menu`] 记下）——否则 `Resize` 动作会把窗口又缩回内容尺寸，
    /// 而 `grow_for_menu` 白撑了一轮，菜单照样被窗口边界裁掉。
    #[must_use]
    pub fn desired_window_size(&self) -> Size {
        if self.zoom_within_window {
            return Size::new(
                self.base_window_size.x.max(1.0) as u32,
                self.base_window_size.y.max(1.0) as u32,
            );
        }
        let margin = self.margin();
        let content = self.content_size();
        // 窗口尺寸**只由内容决定**：内容 × 缩放 + 两侧阴影留白。
        //
        // 这里曾经取 `max(base_window_size)`，于是窗口永远不小于"刚贴出来时的尺寸"——
        // 缩小（滚轮缩小、拖边缩小）只会让图变小、窗口留出一圈空边，拖边看起来就是
        // "拖了没反应"。菜单要的那块由 `menu_extra` 单独兜住，不需要再用基准尺寸兜底。
        let with_menu = Vec2::new(
            content.w as f32 + margin * 2.0,
            content.h as f32 + margin * 2.0,
        )
        .max(self.menu_extra);
        Size::new(
            with_menu.x.round().max(1.0) as u32,
            with_menu.y.round().max(1.0) as u32,
        )
    }

    /// 内容在窗口里的绘制矩形（窗口坐标，egui 点）。
    ///
    /// * 常规模式：内容铺满「窗口去掉阴影留白」的那块区域——即使窗口尺寸因为系统
    ///   响应延迟差了 1 像素，也不会在边缘露出底色；
    /// * `zoom_within_window`：内容按原始缩放居中，超出窗口的部分由窗口裁掉。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        let margin = self.margin();
        // 菜单要的那块（撑大 + 挪窗）不算「窗口」——否则图会被拉长。
        // `menu_window` 是撑大**之前**的窗口内框尺寸（与 `window_size` 同单位，egui 点），
        // 贴图永远只铺满它；菜单没开时是零，这时直接用 `window_size`，与旧行为一致。
        let window = if self.menu_window.x > 0.0 && self.menu_window.y > 0.0 {
            self.menu_window
        } else {
            self.window_size
        };
        // 挪窗的补偿：窗口往上挪了 218，贴图在窗口里就往下挪 218（也就是 `-shift`），
        // 屏幕坐标因此逐像素不变。
        let shift = self.menu_shift;
        let min = Pos2::new(-shift.x, -shift.y);
        let inner = Rect::from_min_size(min, window).shrink(margin);
        // 窗口刚创建、还没跟上目标尺寸时可能比留白还小，此时必须退化成一个
        // 合法矩形而不是负尺寸矩形（负尺寸会让 egui 画出反向的内容）
        if inner.width() < 1.0 || inner.height() < 1.0 {
            return Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 1.0));
        }
        if !self.zoom_within_window {
            return inner;
        }
        let content = self.content_size();
        Rect::from_center_size(
            inner.center(),
            Vec2::new(content.w as f32, content.h as f32),
        )
    }

    /// 诊断用：把菜单期内的摆放状态一行打到日志（标签 `PINDRAW`）。
    fn debug_draw_state(&self, ctx: &egui::Context) {
        let r = self.content_rect();
        log::debug!(
            "PINDRAW outer={:?} vp={:?} win={:?} menu_window={:?} shift={:?} \
             content_min={:?} content_size={:?} anchor={:?}",
            ctx.input(|i| i.viewport().outer_rect.map(|r| r.min)),
            ctx.viewport_rect(),
            self.window_size,
            self.menu_window,
            self.menu_shift,
            r.min,
            r.size(),
            self.menu_anchor,
        );
    }

    /// 为菜单挪动窗口的当前偏移（**egui 点**，往左上为负）。
    ///
    /// 挪窗期间宿主不再判定位置（窗口位置由贴图一手包办），这个方法留给测试和
    /// 以后可能的外部使用者。
    #[must_use]
    pub fn menu_shift(&self) -> Vec2 {
        self.menu_shift
    }

    /// 宿主创建窗口时应使用的尺寸。
    #[must_use]
    pub fn initial_window_size(frame: &Frame, config: &PinConfig) -> Vec2 {
        let margin = if config.shadow { SHADOW_MARGIN } else { 0.0 };
        Vec2::new(
            (frame.size().w as f32 * config.initial_scale).max(1.0) + margin * 2.0,
            (frame.size().h as f32 * config.initial_scale).max(1.0) + margin * 2.0,
        )
    }

    #[must_use]
    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    #[must_use]
    pub fn click_through(&self) -> bool {
        self.click_through
    }

    /// 从外面关掉鼠标穿透（托盘菜单的逃生项）。
    ///
    /// 穿透开着的时候窗口收不到任何鼠标事件，「在菜单里关掉」这条路根本走不通——
    /// 用户点不到那颗开关。所以必须有一条窗口外的路：托盘菜单调这里。
    pub fn set_click_through(&mut self, enabled: bool) {
        self.click_through = enabled;
    }

    #[must_use]
    pub fn always_on_top(&self) -> bool {
        self.always_on_top
    }

    /// 拷贝当前图片（用于「复制」动作）。
    #[must_use]
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// 处理一次滚轮缩放。返回 `true` 表示缩放真的变了。
    ///
    /// 关闭 `wheel_zoom` 时这里**必须**原样返回 `false`——设置里的开关只写不读，
    /// 是这个项目明令禁止的那类假开关。
    fn zoom_by(&mut self, scroll: f32) -> bool {
        if !self.wheel_zoom || scroll.abs() <= 0.1 {
            return false;
        }
        let factor = (scroll * 0.0015).exp();
        let new_scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        if (new_scale - self.scale).abs() <= f32::EPSILON {
            return false;
        }
        self.scale = new_scale;
        self.menu_open = false;
        true
    }

    /// 中键：恢复 100%。返回 `true` 表示缩放真的变了。
    fn reset_zoom(&mut self) -> bool {
        if (self.scale - 1.0).abs() <= f32::EPSILON {
            return false;
        }
        self.scale = 1.0;
        self.menu_open = false;
        true
    }

    /// Ctrl + 滚轮：改不透明度。返回 `true` 表示真的变了（已经到头则 `false`）。
    ///
    /// 和缩放一样走"一格格"的步长而不是指数：不透明度是 10%~100% 的线性量，
    /// 指数曲线在低端几乎不动、在高端又跳得厉害。
    fn adjust_opacity(&mut self, scroll: f32) -> bool {
        if scroll.abs() <= 0.1 {
            return false;
        }
        let step = if scroll > 0.0 {
            OPACITY_WHEEL_STEP
        } else {
            -OPACITY_WHEEL_STEP
        };
        let before = self.opacity;
        self.opacity = (self.opacity + step).clamp(MIN_OPACITY, 1.0);
        if (self.opacity - before).abs() <= f32::EPSILON {
            return false;
        }
        // 和滚轮缩放一样顺手收掉菜单：提示要画在窗口底边，菜单开着就不画了
        self.menu_open = false;
        true
    }

    /// 每帧驱动。返回 `Some` 表示需要宿主响应。
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<PinAction> {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        let mut action = None;

        // 菜单关掉之后，把为菜单撑出来的窗口还原（放在帧首，避免挤掉菜单行自己的动作）
        if !self.menu_open {
            self.shrink_after_menu(ctx, &mut action);
        }

        let screen = ctx.viewport_rect();
        self.window_size = screen.size();

        // 先算好绘制矩形，再借 texture（`texture()` 需要 &mut self）
        let content = self.content_rect();
        let texture = self.texture(ctx).id();
        let painter = ctx.layer_painter(egui::LayerId::new(
            Order::Background,
            Id::new("cuter-pin-image"),
        ));
        self.paint_shadow(&painter, content);

        // 用 `from_white_alpha` 给图片上色调来实现「不透明度」：视口本身是透明的
        // （`ViewportBuilder::with_transparent(true)`），于是写进帧缓冲的 alpha 就是
        // 这里的 alpha，桌面合成器会按它把窗口与背景混合。
        // 注意必须用**预乘**语义的构造：`from_white_alpha(a)` 得到 `(a,a,a,a)`，
        // 正好是「颜色乘 alpha、alpha 也乘 alpha」，与 egui 的预乘混合一致；
        // 若改用 `from_rgba_unmultiplied(255,255,255,a)` 会亮成一团白雾。
        let alpha = (self.opacity.clamp(0.05, 1.0) * 255.0).round() as u8;
        painter.image(
            texture,
            content,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_white_alpha(alpha),
        );
        if self.border {
            painter.rect_stroke(
                content.shrink(0.5),
                CornerRadius::ZERO,
                Stroke::new(1.0, self.theme.accent),
                StrokeKind::Inside,
            );
        }

        // ---- 键鼠输入 ----
        #[allow(clippy::type_complexity)]
        let (
            hover,
            scroll,
            secondary,
            middle,
            ctrl,
            primary_pressed,
            primary_down,
            primary_released,
            middle_down,
            middle_released,
            pointer,
        ) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.smooth_scroll_delta.y,
                i.pointer.button_pressed(PointerButton::Secondary),
                i.pointer.button_pressed(PointerButton::Middle),
                i.modifiers.ctrl,
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.button_down(PointerButton::Primary),
                i.pointer.button_released(PointerButton::Primary),
                i.pointer.button_down(PointerButton::Middle),
                i.pointer.button_released(PointerButton::Middle),
                i.pointer.latest_pos(),
            )
        });


        // ---- 边缘缩放：贴着图边/图角按下时改尺寸，图中间按下才是移动窗口 ----
        //
        // 「只放大内容」模式下窗口尺寸是钉死的，拖边没有意义，所以直接不给缩放区，
        // 保持原来"整窗拖动"的手感。
        let resize_edge = if self.zoom_within_window {
            None
        } else {
            hover.and_then(|p| edge_at_window(content, screen, p))
        };
        if primary_pressed
            && let Some(edge) = resize_edge
        {
            log::debug!(
                "贴图拉边开始：{:?} 起点={:?} 内容={:?} 窗口={:?}",
                edge,
                hover.unwrap_or_default(),
                content,
                screen
            );
            self.resize = Some(PinResize {
                edge,
                start: hover.unwrap_or_default(),
                start_scale: self.scale,
                start_content: content.size(),
            });
        }
        if let Some(edge) = resize_edge
            && self.resize.is_none()
        {
            ctx.set_cursor_icon(edge.cursor());
        }
        if let Some(drag) = self.resize {
            if primary_down {
                if let Some(now) = pointer {
                    let next = scale_after_drag(drag.start_scale, drag.edge, now - drag.start, drag.start_content);
                    log::debug!(
                        "贴图拉边：{:?} 位移={:?} 下一缩放={:.4} 当前={:.4}",
                        drag.edge,
                        now - drag.start,
                        next,
                        self.scale
                    );
                    if (next - self.scale).abs() > f32::EPSILON {
                        self.scale = next;
                        let want = self.desired_window_size();
                        // 往左/往上长时要连带把窗口挪过去，否则抓在手里的那条边会被甩开
                        // （窗口默认钉住左上角，往左拉就变成"右边缘在动"）。
                        let mut at = None;
                        let grows_left = drag.edge.horizontal() && drag.edge.x_sign() < 0.0;
                        let grows_up = drag.edge.vertical() && drag.edge.y_sign() < 0.0;
                        if grows_left || grows_up {
                            // 单位和窗口一致：`desired_window_size` 是图片像素，
                            // `viewport_rect`/`outer_rect` 是 egui 点，换算一次再相减。
                            let ppp = ctx.pixels_per_point().max(0.1);
                            let now_size = ctx.viewport_rect().size();
                            let target = egui::vec2(want.w as f32, want.h as f32) / ppp;
                            let delta = target - now_size;
                            let mut shift = egui::Vec2::ZERO;
                            if grows_left {
                                shift.x = -delta.x;
                            }
                            if grows_up {
                                shift.y = -delta.y;
                            }
                            if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
                                at = Some(outer.min + shift);
                            }
                        }
                        action = Some(PinAction::Resize { size: want, at });
                    }
                }
            } else {
                self.resize = None;
            }
        }

        // ---- 键盘 ----
        let (escape, ctrl_c, ctrl_s) = ctx.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.modifiers.ctrl && i.key_pressed(Key::C),
                i.modifiers.ctrl && i.key_pressed(Key::S),
            )
        });
        if escape {
            return Some(PinAction::Close);
        }
        if ctrl_c {
            return Some(PinAction::Copy);
        }
        if ctrl_s {
            return Some(PinAction::Save);
        }

        // ---- 中键单击：恢复 100%（中键**拖动**是拖出，见下）----
        let mut middle_click_candidate = false;

        // ---- 滚轮：单独滚 = 缩放，Ctrl + 滚 = 不透明度（常见手感）----
        // `zoom_within_window` 时不发任何动作：内容已经在本帧按新缩放重绘，
        // 窗口尺寸保持原样，正是「只放大内容」的含义。
        if ctrl && scroll.abs() > 0.1 {
            if self.adjust_opacity(scroll) {
                let text = fill!(tr!("不透明度 {}%"), format!("{:.0}", self.opacity * 100.0));
                self.toast(ctx, text);
                action = Some(PinAction::SetOpacity(self.opacity));
            }
        } else if self.zoom_by(scroll) && !self.zoom_within_window {
            action = Some(PinAction::Resize { size: self.desired_window_size(), at: None });
        }

        // ---- 右键菜单 ----
        if secondary {
            self.menu_open = !self.menu_open;
            if self.menu_open {
                // 菜单比贴图窗口大：先撑窗口，否则下半截会被窗口边界裁掉
                self.grow_for_menu(ctx, hover.unwrap_or(screen.center()));
                action = Some(PinAction::Resize { size: self.desired_window_size(), at: None });
            }
        }
        if self.menu_open {
            if let Some(a) = self.menu_ui(ctx, hover.unwrap_or(screen.center())) {
                action = Some(a);
            }
            self.debug_draw_state(ctx);
            // 点击菜单之外关闭
            if ctx.input(|i| i.pointer.any_click())
                && hover.is_some_and(|p| !self.menu_rect(screen, hover.unwrap_or(p)).contains(p))
                && !secondary
            {
                self.menu_open = false;
            }
        } else if hover.is_some() && ((ctrl && primary_pressed) || middle) {
            // **Ctrl + 左键拖动**、或**中键拖动** = 把这张图拖到别的程序里（OLE 拖放，宿主负责）。
            //
            // 两个手势都给：`Ctrl+左键` 是顺手的，但它依赖窗口有键盘焦点
            // （egui 的 `modifiers` 来自焦点窗口的按键事件）；中键不依赖任何修饰键，
            // 所以在"贴图没被激活"的情况下也一定能用。
            //
            // 先只记下起点：一按下就起拖会把手抖当拖拽，必须先移动超过阈值。
            self.drag_out_armed = pointer.or(hover);
            self.drag_out_middle = middle;
            middle_click_candidate = middle;
        } else if hover.is_some() && primary_pressed && self.resize.is_none() {
            // 用系统的窗口拖拽，避免自己实现与 DPI 相关的位移换算
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        // 已经按下：移动超过阈值就起拖（只发一次）
        if let Some(origin) = self.drag_out_armed {
            let still_held = if self.drag_out_middle {
                middle_down
            } else {
                ctrl && primary_down
            };
            if !still_held {
                // 松手了但没超过阈值：当成普通点击
                self.drag_out_armed = None;
            } else if let Some(now) = pointer
                && (now - origin).length() > DRAG_OUT_THRESHOLD
            {
                log::debug!("贴图起拖：距离 {:.1}（{}-drag）", (now - origin).length(), if self.drag_out_middle { "middle" } else { "ctrl+left" });
                // 拖成了：这一下就不是"中键单击"，不该再恢复缩放
                self.drag_out_armed = None;
                self.drag_out_middle = false;
                middle_click_candidate = false;
                action = Some(PinAction::DragOut);
            }
        }
        // 中键单击（没拖成）→ 恢复 100%
        if middle_released
            && middle_click_candidate
            && self.reset_zoom()
            && !self.zoom_within_window
        {
            action = Some(PinAction::Resize { size: self.desired_window_size(), at: None });
        }
        let _ = primary_released;

        // 按住 Ctrl 悬停时给一句提示：这个手势不看文档是猜不到的
        if ctrl && hover.is_some() && self.drag_out_armed.is_none() {
            ctx.layer_painter(egui::LayerId::new(
                Order::Tooltip,
                Id::new("cuter-pin-drag-hint"),
            ))
            .text(
                screen.center_top() + Vec2::new(0.0, 6.0),
                Align2::CENTER_TOP,
                tr!("拖动可以把这张图拖到别的程序里"),
                egui::FontId::proportional(12.0),
                self.theme.accent,
            );
        }
        // 一次性提示到点就撤：它是"回应刚才那一下"，看完就该走味，
        // 留在窗口底边会一直挡着图。
        if let Some(until) = self.status_until {
            let now = ctx.input(|i| i.time);
            if now >= until {
                self.status = None;
                self.status_until = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(until - now));
            }
        }
        // 菜单开着时不画提示：菜单展开后窗口底边落到菜单行里头，照旧画在底边会压在
        // 「关闭贴图」那一行上（实测提示糊在最后一行）。菜单本身就自带状态：
        // 开关的开/关、不透明度的百分比都在行里写着。
        if let Some(status) = &self.status
            && !self.menu_open
        {
            ctx.layer_painter(egui::LayerId::new(
                Order::Tooltip,
                Id::new("cuter-pin-status"),
            ))
            .text(
                screen.center_bottom() - Vec2::new(0.0, 18.0),
                Align2::CENTER_BOTTOM,
                status,
                egui::FontId::proportional(13.0),
                Color32::WHITE,
            );
        }

        action
    }

    /// 一句过几秒自己消失的提示（滚轮调了不透明度、开关了穿透/置顶）。
    pub fn toast(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.status = Some(text.into());
        self.status_until = Some(ctx.input(|i| i.time) + TOAST_SECONDS);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(TOAST_SECONDS));
    }

    fn paint_shadow(&self, painter: &egui::Painter, content: Rect) {
        if !self.shadow {
            return;
        }
        // 由外向内叠：最外圈只叠到一次，越靠近图片叠得越多，于是自然形成渐变。
        // 图片随后会盖住中心区域，最终只留下四周的投影。
        //
        // 每层的圆角跟着图片走（图片圆角 + 外扩量）：圆角写死成 6 的时候，
        // 图片圆角 20+ 的贴图四角会露出一圈"方"的灰边，看着像相框。
        let radius = self.corner_radius() as f32;
        let step = SHADOW_MARGIN / SHADOW_LAYERS as f32;
        for layer in 0..SHADOW_LAYERS {
            let grow = SHADOW_MARGIN - layer as f32 * step;
            let corner = (radius + grow).round().clamp(0.0, 255.0) as u8;
            painter.rect_filled(
                content.expand(grow).translate(Vec2::new(0.0, SHADOW_DROP)),
                CornerRadius::same(corner),
                Color32::from_black_alpha(SHADOW_ALPHA),
            );
        }
    }

    /// 圆角半径：按图片短边自适应（6%，夹在 2~24 之间）。整像素，避免小图上出现怪半径。
    fn corner_radius(&self) -> u32 {
        if !self.rounded {
            return 0;
        }
        let short = self.frame.width().min(self.frame.height());
        (short * 6 / 100).clamp(2, 24)
    }

    /// 改圆角开关（设置里改了立即生效）：纹理要重建，所以直接丢掉。
    pub fn set_rounded(&mut self, rounded: bool) {
        if self.rounded != rounded {
            self.rounded = rounded;
            self.texture = None;
        }
    }


    /// 当前持有的纹理占多少字节（RGBA8，宽×高×4）。没建纹理就是 0。
    ///
    /// 贴图纹理是**惰性**创建的，所以这个数只在它真的显示过之后才非零。
    #[must_use]
    pub fn texture_bytes(&self) -> usize {
        if self.texture.is_none() {
            return 0;
        }
        self.frame.width() as usize * self.frame.height() as usize * 4
    }

    /// 丢掉纹理（原始 `Frame` 留着）。
    ///
    /// 用途：贴图被分组切走、或者整体隐藏时——那些窗口不再声明，gpui 也不会跑它们的 pass，
    /// 于是纹理既没人用、又一直占着显存与内存。下次它重新显示时 [`Self::texture`] 会重建，
    /// 代价只是一次上传。
    pub fn drop_texture(&mut self) {
        self.texture = None;
    }

    fn texture(&mut self, ctx: &egui::Context) -> &egui::TextureHandle {
        if self.texture.is_none() {
            // 圆角在 CPU 上裁一次（只在上传纹理时做，之后每帧都是同一张纹理）。
            let radius = self.corner_radius();
            let frame = if radius == 0 {
                std::borrow::Cow::Borrowed(&self.frame)
            } else {
                std::borrow::Cow::Owned(cuter_core::frame::rounded_corners(&self.frame, radius))
            };
            let image = frame_to_color_image(&frame);
            self.texture = Some(ctx.load_texture("cuter-pin", image, egui::TextureOptions::NEAREST));
        }
        self.texture.as_ref().expect("刚刚写入过")
    }

    /// 右键菜单比贴图窗口大时，先把窗口撑到装得下菜单。
    ///
    /// 撑出来的那块是**透明**的：`content_rect` 会按 `menu_extra` 把它折回去，
    /// 所以贴图本身既不会移位也不会被拉伸。菜单关掉后由 [`Self::shrink_after_menu`] 还原。
    ///
    /// 光撑窗口在屏幕边上是没用的：窗口尺寸会被显示器工作区夹住（实测贴屏幕右下角时
    /// 264×144 只撑到 330×180），菜单照样露不全。所以要**同时挪窗口**——
    /// 把窗口挪到「菜单整个落在显示器里」，挪出来的窟窿由 `menu_extra` 补上，
    /// 贴图在屏幕上的位置则靠 [`Self::menu_shift`] 补偿回来。
    /// 菜单挪窗时窗口内框左上角要放到的位置（egui 点，绝对屏幕坐标）。
    ///
    /// 宿主用它把 `GeometryFix::target_pos` 对齐到同一个位置，免得两边各发一次
    /// `OuterPosition` 互相覆盖（实验里就出现过宿主把窗口拖回原位、菜单又被裁掉）。
    pub fn menu_target_origin(&self) -> Option<Pos2> {
        self.menu_pos_restore
    }

    /// 菜单挪窗期间窗口最多需要多大（egui 点）；没开菜单时返回 [`Vec2::ZERO`]。
    pub fn menu_target_size(&self) -> Vec2 {
        self.menu_extra
    }

    fn grow_for_menu(&mut self, ctx: &egui::Context, at: Pos2) {
        if self.menu_restore.is_some() {
            return; // 这一次菜单已经撑过了
        }
        let current = Vec2::new(self.window_size.x.max(1.0), self.window_size.y.max(1.0));
        let outer_now = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
        let monitor_size = ctx.input(|i| i.viewport().monitor_size);

        // 摆放计算一律用 **egui 点**：`at`（视口内偏移）、`outer`（`outer_rect.min`）、
        // `monitor`（`monitor_size`）、`current`（`window_size`）全都已经是点，
        // 而 `MENU_WIDTH`/`MENU_HEIGHT`/`MENU_PAD` 也是点。
        //
        // ⚠️ 把点击点在**挪动之前**的视口坐标冻结下来（`menu_at`）：
        // 窗口会被贴图与宿主各挪一次，而 `at` 是「相对当前窗口」量的，窗口一挪它就跟着变。
        // 每帧拿新的 `at` 当输入去算摆放，等于把已经挪过的位移又算一遍，窗口会一路
        // 挪过头（实测挪了 2 倍，菜单只剩第一行）。
        if self.menu_at.is_none() {
            self.menu_at = Some(at - ctx.viewport_rect().min);
        }
        let at = self.menu_at.unwrap_or_else(|| at - ctx.viewport_rect().min);
        let (shift, need, anchor) = match (outer_now, monitor_size) {
            (Some(outer), Some(monitor)) if monitor.x > 0.0 && monitor.y > 0.0 => {
                let placed = menu_placement(at, outer, current, monitor);
                // `anchor` 是窗口内坐标，与 `menu_rect` 同一把尺子。
                // 位置按**绝对坐标**下发：窗口会被贴图和宿主各挪一次，用偏移会叠加成两倍。
                // `OuterPosition` 收的就是 egui 点，这里不需要 ppp 换算。
                self.send_menu_shift(ctx, placed.origin_physical, placed.shift_physical);
                (placed.shift_physical, placed.need_physical, placed.anchor_physical)
            }
            _ => {
                let need = self.need_for_menu(at.to_pos2());
                self.send_menu_shift(ctx, Pos2::new(0.0, 0.0), Vec2::ZERO);
                (Vec2::ZERO, need, at.to_pos2())
            }
        };
        if need.x <= current.x && need.y <= current.y {
            // 装得下：窗口原地不动，菜单还画在点击点，`content_rect` 走原路。
            self.menu_anchor = Some(at.to_pos2());
            self.menu_window = current;
            return;
        }
        // 记下撑大前的窗口尺寸：`content_rect` 要按它摆贴图，才不会把菜单撑出来那块
        // 算进贴图里（贴图既不能挪位、也不能被拉长）。
        self.menu_window = current;
        self.menu_extra = Vec2::new((need.x - current.x).max(0.0), (need.y - current.y).max(0.0));
        // 菜单是**贴着点击点 `at`** 画的，可窗口一挪，屏幕上同一处点在视口里的坐标就变了。
        // 把挪完之后的坐标记下来交给 `menu_rect`，免得菜单跑到窗口外被裁掉。
        self.menu_anchor = Some(anchor);
        // `menu_shift` 与 `window_size` 同为 egui 点：`content_rect` 要在同一坐标里补偿绘制。
        self.menu_shift = shift;
        self.menu_restore = Some(self.base_window_size);
        // 菜单要的是**整块**尺寸（贴图 + 菜单都装下），不是增量：`desired_window_size`
        // 拿 `base_window_size` 当最小值去和 `content + margin` 比，只塞增量的话它会取
        // 后者，窗口压根不长（实测宽度停在 264 点，菜单右边被裁掉）。
        self.base_window_size = need;
    }

    /// 菜单画在 `at` 处时要多大窗口才装得下（`at` 是视口坐标）。
    fn need_for_menu(&self, at: Pos2) -> Vec2 {
        Vec2::new(
            at.x + MENU_WIDTH + MENU_PAD,
            at.y + MENU_HEIGHT + MENU_PAD,
        )
    }

    /// 把窗口挪到 `origin`（窗口内框左上角的**绝对**屏幕坐标，egui 点）。
    ///
    /// 传绝对坐标而不是偏移：窗口会被贴图（这里）和宿主（`GeometryFix`）各下发布局一次，
    /// 用「当前 `outer_rect` + 偏移」会让两次位移叠加，菜单反而跑出屏幕。
    fn send_menu_shift(&mut self, ctx: &egui::Context, origin: Pos2, shift: Vec2) {
        if self.menu_pos_restore.is_none() {
            self.menu_pos_restore = Some(origin);
        }
        if shift == self.menu_shift_sent {
            return; // 已经下发过，别每帧都发（会打断用户手动拖动窗口）
        }
        self.menu_shift_sent = shift;
        // `OuterPosition` 传的是「egui 点」：egui-winit 会自己乘 `pixels_per_point`
        // （egui-winit/src/lib.rs:1803），而 `outer_rect` 也是点，两边同一把尺子。
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(origin));
    }

    /// 菜单关掉之后还原窗口尺寸**和位置**。
    fn shrink_after_menu(&mut self, ctx: &egui::Context, action: &mut Option<PinAction>) {
        let Some(restore) = self.menu_restore.take() else {
            return;
        };
        if let Some(origin) = self.menu_pos_restore.take() {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(origin));
        }
        // 位置和尺寸要**同一帧**还原：先归零偏移，`content_rect` 当场按"还在原位"
        // 画，而窗口下一帧就真的回去了（否则会闪一帧错位的贴图）。
        self.menu_shift = Vec2::ZERO;
        self.menu_shift_sent = Vec2::ZERO;
        self.menu_at = None;
        self.menu_anchor = None;
        self.menu_window = Vec2::ZERO;
        self.base_window_size = restore;
        self.menu_extra = Vec2::ZERO;
        *action = Some(PinAction::Resize { size: self.desired_window_size(), at: None });
    }

    fn menu_rect(&self, screen: Rect, at: Pos2) -> Rect {
        // 窗口挪过之后菜单要跟着挪（见 `menu_anchor`），否则它会留在窗口外被裁掉。
        let at = self.menu_anchor.unwrap_or(at);
        let size = Vec2::new(MENU_WIDTH, MENU_HEIGHT);
        let mut pos = at;
        if pos.x + size.x > screen.right() {
            pos.x = screen.right() - size.x;
        }
        if pos.y + size.y > screen.bottom() {
            pos.y = screen.bottom() - size.y;
        }
        let rect = Rect::from_min_size(pos.max(screen.min), size);
        log::debug!(
            "MENURECT rect={rect:?} anchor_at={at:?} screen={screen:?} shift={:?} target={:?}",
            self.menu_shift,
            self.menu_pos_restore,
        );
        rect
    }

    fn menu_ui(&mut self, ctx: &egui::Context, at: Pos2) -> Option<PinAction> {
        let screen = ctx.viewport_rect();
        let rect = self.menu_rect(screen, at);
        let theme = self.theme;
        let mut action = None;
        let mut close = false;

        Area::new(Id::new("cuter-pin-menu"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme.surface)
                    .stroke(Stroke::new(1.0, theme.outline))
                    .corner_radius(CornerRadius::same(MENU_CORNER))
                    .inner_margin(egui::Margin::same(MENU_PADDING))
                    .show(ui, |ui| {
                        ui.set_width(rect.width() - f32::from(MENU_PADDING) * 2.0);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;

                        let (_, response) = menu_row(ui, &theme, tr!("复制"), Some("Ctrl+C"), false);
                        if response.clicked() {
                            action = Some(PinAction::Copy);
                            close = true;
                        }
                        let (_, response) = menu_row(ui, &theme, tr!("保存为…"), Some("Ctrl+S"), false);
                        if response.clicked() {
                            action = Some(PinAction::Save);
                            close = true;
                        }
                        let (_, response) = menu_row(ui, &theme, tr!("识别文字"), Some("Ctrl+O"), false);
                        if response.clicked() {
                            action = Some(PinAction::Ocr);
                            close = true;
                        }

                        menu_separator(ui, &theme);

                        let (row, _) = menu_row(
                            ui,
                            &theme,
                            tr!("不透明度"),
                            Some(&format!("{:.0}%", self.opacity * 100.0)),
                            false,
                        );
                        let slider = Rect::from_min_size(
                            Pos2::new(
                                row.right() - MENU_PAD_X - SLIDER_HINT_GAP - SLIDER_WIDTH,
                                row.center().y - 9.0,
                            ),
                            Vec2::new(SLIDER_WIDTH, 18.0),
                        );
                        if ui
                            .put(
                                slider,
                                egui::Slider::new(&mut self.opacity, MIN_OPACITY..=1.0).show_value(false),
                            )
                            .changed()
                        {
                            action = Some(PinAction::SetOpacity(self.opacity));
                        }

                        // 开关行**整行**都能点：只让右端那颗小胶囊可点的话，用户点行里的
                        // 文字（「始终置顶」）什么也不会发生，看起来就是"点了没反应"。
                        let (row, response) = menu_row(ui, &theme, tr!("鼠标穿透"), None, false);
                        if menu_toggle(ui, &theme, row, &response, &mut self.click_through) {
                            action = Some(PinAction::ToggleClickThrough(self.click_through));
                        }
                        let (row, response) = menu_row(ui, &theme, tr!("始终置顶"), None, false);
                        if menu_toggle(ui, &theme, row, &response, &mut self.always_on_top) {
                            self.toast(
                                ui.ctx(),
                                if self.always_on_top {
                                    tr!("已置顶：这张图会浮在别的窗口上面")
                                } else {
                                    tr!("已取消置顶：别的窗口会盖住这张图，右键菜单可再开启")
                                },
                            );
                            action = Some(PinAction::SetAlwaysOnTop(self.always_on_top));
                        }

                        menu_separator(ui, &theme);

                        let info = format!(
                            "{} × {}  ·  {:.0}%{}",
                            self.frame.size().w,
                            self.frame.size().h,
                            self.scale * 100.0,
                            if self.zoom_within_window {
                                tr!("  ·  窗口内缩放").to_string()
                            } else {
                                String::new()
                            }
                        );
                        menu_row(ui, &theme, &info, None, false);

                        menu_separator(ui, &theme);

                        let (_, response) = menu_row(ui, &theme, tr!("关闭贴图"), Some("Esc"), false);
                        if response.clicked() {
                            action = Some(PinAction::Close);
                            close = true;
                        }
                    });
            });

        if close {
            self.menu_open = false;
        }
        action
    }
}

/// 为「右键菜单整个可见」算出来的窗口摆放方案。
///
/// 全部是**物理像素**（`Vec2`/`Pos2` 而不是 egui 点），调用方自己乘 `pixels_per_point`。
#[derive(Debug, Clone, Copy)]
struct MenuPlacement {
    /// 窗口内框左上角在屏幕上应该放到的位置（物理像素，绝对值）。
    ///
    /// 存**绝对位置**而不是偏移，是因为窗口会被挪两次——贴图自己发一次
    /// `OuterPosition`，宿主的 `GeometryFix` 又按目标位置下发一次。拿「上一帧的
    /// `outer_rect`」当基准算偏移，两次就叠加成了两倍位移，菜单反而跑出屏幕
    /// （实测窗口挪到 1220.8 点，菜单右沿 2038 物理像素，屏幕上只看得见第一行）。
    origin_physical: Pos2,
    /// 相对原始位置挪了多少（往左/上为负，零表示不用挪），给宿主并进几何目标用。
    shift_physical: Vec2,
    /// 挪完之后，窗口内框至少要多大的物理像素才装得下贴图**和**菜单。
    ///
    /// 两项取大：贴图在窗口里的落点是 `-shift`（窗口上移多少，贴图就得往下让多少），
    /// 底边 `original - shift`；菜单在窗口里的落点是 `at - origin`（`at` 是屏幕坐标），
    /// 底边 `at - origin + menu + pad`。
    need_physical: Vec2,
    /// 挪完之后，菜单左上角在**窗口内**的位置（物理像素）。
    ///
    /// 就是 `menu_rect` 要画的地方：屏幕位置 `at_screen` 减去新窗口原点 `origin`。
    /// 恒为非负——窗口最多挪到让菜单左上角贴住屏幕左上角。
    anchor_physical: Pos2,
}

/// 菜单装不下时窗口要多大、要挪多少。
///
/// 全部是 **egui 点**，和 `ctx.viewport_rect()` / `outer_rect` / `monitor_size` 同一把尺子；
/// [`MENU_WIDTH`]/[`MENU_HEIGHT`]/[`MENU_PAD`] 本来就是点，不需要再乘 `pixels_per_point`。
///
/// * `local`：菜单左上角相对**窗口原点**的偏移（点，= 点击点 − 窗口原点）；
/// * `outer`：窗口内框左上角**当前**在屏幕上的坐标（点）；
/// * `original`：当前窗口内框尺寸（点）；
/// * `monitor`：**显示器**尺寸（点）。
///
/// 关键事实：菜单是 `Area::fixed_pos` 画在**窗口视口**里的，它的屏幕位置是
/// `origin + anchor`（`anchor` 是菜单在窗口里的落点）。窗口往左上让 `shift`（`origin = outer + shift`）
/// 的同时，把菜单的落点从 `local` 改成 `local - shift`，屏幕位置就是
/// `outer + shift + local - shift = outer + local` —— **菜单的屏幕位置不随窗口移动**，
/// 它就钉在点击点上。所以「挪窗口救菜单」本身不成立：菜单一开始就在点击点，它没跑。
///
/// 真正要解决的是**窗口放不下菜单**：点击点在窗口右下角时，`local + menu + pad` 超出了
/// 原来 264×144 的窗口，`menu_rect` 只好把菜单裁到窗口底/右边。于是这里只做一件事——
/// **挪窗口 + 补偿绘制偏移**的求解器。
///
/// 输入/输出全是 **egui 点**（`local` 是点击点在窗口内的偏移，`outer` 是窗口原点，
/// 两者不同尺子 —— 不要相减）。
///
/// 关键认识：菜单是画在**窗口视口**里的（`Area::fixed_pos`），屏幕落点 = `outer + local`。
/// 挪窗口时同一屏幕点的视口坐标变成 `local - shift`，屏幕落点 `outer + shift + (local - shift)`
/// 不变 —— 所以**挪窗口本身不可能把菜单挪进屏幕**；真正把菜单裁掉的是窗口自己（菜单超出
/// 窗口视口的部分被裁）。因此只有两条路：
///
/// 1. **撑大**窗口到同时装得下「原位不动的贴图」和「菜单」；
/// 2. 菜单贴到屏幕边时按屏幕**夹**一下落点，再只往左上**让**窗口，让完后窗口自身别顶出屏幕。
///
/// 菜单和贴图各自都要装得进窗口：
///
/// ```text
/// anchor + menu + pad <= need      菜单整块在窗口内（anchor = screen - origin）
/// original - shift   <= need      贴图整块在窗口内（shift <= 0，所以是往右下补）
/// origin + need      <= work      窗口自身右/下沿不出屏幕
/// ```
///
/// 其中 `need = max(original, screen - origin + menu + pad)`，代入第一、二条都恰好成立；
/// 剩下就只有 `origin + need <= work` 与 `origin <= outer`，逐轴求解见 [`axis_fit`]。
fn menu_placement(local: Vec2, outer: Pos2, original: Vec2, monitor: Vec2) -> MenuPlacement {
    let menu = Vec2::new(MENU_WIDTH, MENU_HEIGHT);
    // 菜单右/下沿留 [`MENU_PAD`] 余量：窗口外框比内框略大、菜单还有描边和投影。
    let pad = MENU_PAD;
    let work = Vec2::new(monitor.x.max(1.0), monitor.y.max(1.0));

    // 1) 菜单的**屏幕**落点：钉在点击处 `outer + local`，贴到屏幕边就沿边贴住。
    //    屏幕比菜单还小时 `work - menu - pad` 为负，夹完菜单跑到屏幕上沿外侧，这时
    //    「能露多少露多少」——多出来的一截由窗口/屏幕裁掉。
    let screen = (outer.to_vec2() + local).min(work - menu - Vec2::splat(pad));

    // 2) 逐轴求窗口原点：菜单想钉在屏幕 `screen` 处，贴图仍在屏幕 `outer` 处。窗口原点
    //    `origin` 同时决定两者的窗口内落点（菜单 `screen - origin`、贴图 `outer - origin`），
    //    约束与取法见 [`axis_fit`] —— 放得下时一步都不挪，放不下才往左上让。
    let (origin_x, need_x) = axis_fit(screen.x, local.x, menu.x + pad, original.x, outer.x);
    let (origin_y, need_y) = axis_fit(screen.y, local.y, menu.y + pad, original.y, outer.y);

    // 3) 菜单在窗口内的落点 = 屏幕落点 - 窗口原点。窗口让出去多少，这里就补回来多少，
    //    所以菜单在屏幕上纹丝不动。
    let origin = Vec2::new(origin_x, origin_y);
    let anchor = screen - origin;
    let shift = origin - outer.to_vec2();

    // 4) 窗口尺寸：贴图落进窗口的 `-shift` 处（`content_rect` 会照此补偿），
    //    菜单落进 `anchor` 处 —— 两者都装得下。
    let need = Vec2::new(need_x, need_y);

    MenuPlacement {
        origin_physical: origin.to_pos2(),
        shift_physical: shift,
        need_physical: need,
        anchor_physical: anchor.to_pos2(),
    }
}


/// 单轴求解：返回 `(窗口原点, 窗口尺寸)`。
///
/// `screen` 是菜单**想落**的屏幕坐标（还没夹屏幕），`menu` 已含 [`MENU_PAD`] 留白，
/// `local` 是点击点在窗口内的偏移，`original`/`outer` 是贴图尺寸与窗口内框原点。
/// 窗口原点 `origin` 一旦离开 `outer`，贴图在窗口里的落点就要往右下补 `outer - origin`
/// （`content_rect` 负责），菜单的窗口内落点则是 `screen - origin`。于是：
///
/// ```text
/// origin = clamp(screen - local, 0, outer)     菜单钉在屏幕上，窗口只往左上让
/// need   = max(original - (origin - outer), (screen - origin) + menu)
/// ```
///
/// `original - (origin - outer)` 是贴图让位后需要的宽度（`outer + original - origin`），
/// 另一项是菜单右/下沿。窗口的无边框 + 全透明区域允许伸到屏幕外，所以窗口自身不必
/// 整块进屏 —— 真正要进屏的是菜单。屏幕连菜单都装不下时 `origin` 落到 0、
/// `menu_rect` 再把露不出的那截夹回窗口内，能露多少露多少，绝不 panic。
fn axis_fit(screen: f32, local: f32, menu: f32, original: f32, outer: f32) -> (f32, f32) {
    let origin = (screen - local).clamp(0.0, outer);
    let need = (original - (origin - outer))
        .max(screen - origin + menu)
        .max(1.0);
    (origin, need)
}


/// 菜单一行：左侧标签、右侧快捷键提示；hover 用 `accent/subtle` 高亮（圆角 8）。
fn menu_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    hint: Option<&str>,
    danger: bool,
) -> (Rect, egui::Response) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, MENU_ROW_HEIGHT),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), theme.accent_subtle);
    }
    ui.painter().text(
        Pos2::new(rect.left() + MENU_PAD_X, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if danger { theme.danger } else { theme.on_surface },
    );
    if let Some(hint) = hint {
        ui.painter().text(
            Pos2::new(rect.right() - MENU_PAD_X, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            hint,
            egui::FontId::proportional(11.0),
            theme.on_surface_faint,
        );
    }
    (rect, response)
}

/// 菜单里的开关：28×16 胶囊 + 白色圆钮，**整行**点一下都翻转（返回是否变化）。
///
/// `row_response` 是 `menu_row` 给的那一整行的可点区域。以前只认胶囊自己那 28 点宽，
/// 用户点行里的「始终置顶」四个字是没反应的——本机探针按 (锚点+68, +225) 打过去连
/// `WS_EX_TRANSPARENT` 都没变，就是踩了这个坑。
fn menu_toggle(
    ui: &mut egui::Ui,
    theme: &Theme,
    row: Rect,
    row_response: &egui::Response,
    on: &mut bool,
) -> bool {
    let size = Vec2::new(28.0, 16.0);
    let track = Rect::from_min_size(
        Pos2::new(row.right() - MENU_PAD_X - size.x, row.center().y - size.y / 2.0),
        size,
    );
    // 胶囊自己也要可点（鼠标停在钮上时指针该变成手型），但**判定**用整行：
    // 两者命中同一个点击时只翻转一次（`||` 短路）。
    let switch = ui.interact(
        track,
        ui.id().with(("pin-menu-toggle", (row.top() * 10.0) as i32)),
        egui::Sense::click(),
    );
    let fill = if *on { theme.accent } else { theme.surface_sunken };
    ui.painter().rect_filled(track, CornerRadius::same(8), fill);
    let knob_x = if *on {
        track.right() - 8.0
    } else {
        track.left() + 8.0
    };
    ui.painter()
        .circle_filled(Pos2::new(knob_x, track.center().y), 6.0, egui::Color32::WHITE);
    if switch.clicked() || row_response.clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// 菜单里的分隔线：1px `outline`，上下各留 4px。
fn menu_separator(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), MENU_SEPARATOR_HEIGHT),
        egui::Sense::hover(),
    );
    let y = rect.center().y.round() + 0.5;
    ui.painter().line_segment(
        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
        Stroke::new(1.0, theme.outline),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::filled(Size::new(100, 50), [255, 0, 0, 255])
    }

    fn pin_with(mutate: impl FnOnce(&mut PinConfig)) -> Pin {
        let mut config = PinConfig {
            shadow: false,
            ..PinConfig::default()
        };
        mutate(&mut config);
        let window = Pin::initial_window_size(&frame(), &config);
        Pin::new(frame(), &config, Theme::dark(), window)
    }

    /// 阴影留白是「窗口比内容大出来的那一圈」，不能挤占内容本身。
    #[test]
    fn shadow_margin_only_grows_the_window() {
        let plain = pin_with(|_| {});
        assert_eq!(plain.desired_window_size(), Size::new(100, 50));
        assert_eq!(
            plain.content_rect(),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 50.0))
        );

        let shadowed = pin_with(|c| c.shadow = true);
        assert_eq!(shadowed.margin(), SHADOW_MARGIN);
        assert_eq!(
            shadowed.desired_window_size(),
            Size::new(
                100 + (SHADOW_MARGIN * 2.0) as u32,
                50 + (SHADOW_MARGIN * 2.0) as u32
            )
        );
        // 内容矩形必须仍然恰好是图片尺寸，并且四周各留出 margin
        let rect = shadowed.content_rect();
        assert_eq!(rect.width(), 100.0);
        assert_eq!(rect.height(), 50.0);
        assert_eq!(rect.min, Pos2::new(SHADOW_MARGIN, SHADOW_MARGIN));
    }

    /// `pin.wheel_zoom = false` 时滚轮必须完全不改变缩放（假开关回归测试）。
    #[test]
    fn wheel_zoom_off_ignores_the_scroll_wheel() {
        let mut pinned = pin_with(|c| c.wheel_zoom = false);
        let before = pinned.content_size();
        assert!(!pinned.zoom_by(120.0));
        assert_eq!(pinned.content_size(), before);

        let mut zoomable = pin_with(|_| {});
        assert!(zoomable.zoom_by(120.0));
        assert!(zoomable.content_size().w > before.w);
    }

    /// `pin.zoom_within_window`：缩放只改内容，窗口尺寸钉死。
    #[test]
    fn zoom_within_window_pins_the_window_size() {
        let mut pinned = pin_with(|c| {
            c.shadow = true;
            c.zoom_within_window = true;
        });
        let window = pinned.desired_window_size();
        assert!(pinned.zoom_by(200.0));
        assert!(pinned.content_size().w > 100);
        assert_eq!(
            pinned.desired_window_size(),
            window,
            "窗口内缩放模式下发出去的窗口尺寸必须恒定"
        );
        // 内容变大后仍然以窗口中心为中心
        let inner = Rect::from_min_size(Pos2::ZERO, pinned.window_size).shrink(SHADOW_MARGIN);
        assert!((pinned.content_rect().center() - inner.center()).length() < 0.01);
        assert!(
            pinned.content_rect().width() > inner.width(),
            "内容应当溢出窗口并被裁掉"
        );
    }

    /// 关掉 `zoom_within_window` 时窗口要跟着内容走。
    #[test]
    fn plain_mode_resizes_the_window_with_the_content() {
        let mut pinned = pin_with(|_| {});
        assert!(pinned.zoom_by(200.0));
        let size = pinned.desired_window_size();
        assert_eq!(size, pinned.content_size());
        assert!(size.w > 100);
    }

    #[test]
    fn middle_click_restores_one_hundred_percent() {
        let mut pinned = pin_with(|_| {});
        pinned.zoom_by(300.0);
        assert!(pinned.reset_zoom());
        assert_eq!(pinned.content_size(), Size::new(100, 50));
        // 已经是 100% 时不应重复下发尺寸命令
        assert!(!pinned.reset_zoom());
    }

    #[test]
    fn initial_window_size_accounts_for_the_shadow() {
        let config = PinConfig {
            shadow: true,
            initial_scale: 2.0,
            ..PinConfig::default()
        };
        let size = Pin::initial_window_size(&frame(), &config);
        assert_eq!(
            size,
            Vec2::new(200.0 + SHADOW_MARGIN * 2.0, 100.0 + SHADOW_MARGIN * 2.0)
        );
    }

    /// 屏幕装得下时，`menu_placement` 必须原地不动（否则菜单一开窗口就乱跳）。
    #[test]
    fn menu_placement_keeps_a_roomy_window_in_place() {
        // 全是 egui 点：窗口 264×144 在 (100,100)，点击点相对窗口原点 (132,72)，屏幕 1920×1080。
        let placement = menu_placement(
            Vec2::new(132.0, 72.0),
            Pos2::new(100.0, 100.0),
            Vec2::new(264.0, 144.0),
            Vec2::new(1920.0, 1080.0),
        );
        assert_eq!(placement.shift_physical, Vec2::ZERO);
        assert_eq!(placement.anchor_physical, Pos2::new(132.0, 72.0));
        assert_eq!(
            placement.need_physical,
            Vec2::new(132.0 + MENU_WIDTH + MENU_PAD, 72.0 + MENU_HEIGHT + MENU_PAD)
        );
    }

    /// 贴屏幕右下角时：窗口往左上让到能装下整块菜单，再撑到装得下菜单（贴图仍钉在原处）。
    ///
    /// 数字用**实测口径**：贴图窗口 264×144 物理像素摆在工作区右下角，显示器 1920×1080 物理
    /// = 1536×864 egui 点（ppp = 1.25），所以窗口内框原点 (1324.8, 718.4) 点、尺寸
    /// (211.2, 115.2) 点，右键落在窗口中心即 (105.6, 56.8) 点，菜单 200×288 点 + 10 点留白。
    #[test]
    fn menu_placement_grows_the_window_in_place_at_the_bottom_right_corner() {
        let outer = Pos2::new(1324.8, 718.4);
        let original = Vec2::new(211.2, 115.2);
        let monitor = Vec2::new(1536.0, 864.0);
        let local = Vec2::new(105.6, 56.8);
        let placement = menu_placement(local, outer, original, monitor);

        // 横向：菜单屏幕落点被屏幕右沿夹到 `1326`，窗口要让到 `1326 - 105.6 = 1220.4`
        // （`shift.x = 1220.4 - 1324.8 = -104.4`）；纵向：菜单屏幕落点夹到 `566`，
        // 窗口让到 `566 - 56.8 = 509.2`（`shift.y = -209.2`）。
        assert!((placement.shift_physical.x + 104.4).abs() < 0.01);
        assert!((placement.shift_physical.y + 209.2).abs() < 0.01);
        // 菜单屏幕落点 `1326 / 566` 减窗口原点 `1220.4 / 509.2`。
        assert!((placement.anchor_physical.x - 105.6).abs() < 0.01);
        assert!((placement.anchor_physical.y - 56.8).abs() < 0.01);
        // `need` = max(贴图让位后的右/下沿, 菜单右/下沿)：贴图 `211.2 + 104.4 = 315.6`
        // 对菜单 `105.6 + 210 = 315.6` ⇒ 315.6；纵向 `115.2 + 209.2 = 324.4` 对
        // `56.8 + 298 = 354.8` ⇒ 354.8。
        assert!((placement.need_physical.x - 315.6).abs() < 0.01);
        assert!((placement.need_physical.y - 354.8).abs() < 0.01);

        // 菜单整块落进窗口内框（锚点 + 菜单本体 + 留白）。
        assert!(
            placement.anchor_physical.x + MENU_WIDTH + MENU_PAD <= placement.need_physical.x + 0.01
        );
        assert!(
            placement.anchor_physical.y + MENU_HEIGHT + MENU_PAD <= placement.need_physical.y + 0.01
        );
        // 菜单右/下沿进屏。
        assert!(outer.x + placement.shift_physical.x + local.x + MENU_WIDTH + MENU_PAD <= monitor.x);
        assert!(
            outer.y + placement.shift_physical.y + local.y + MENU_HEIGHT + MENU_PAD <= monitor.y
        );
        // 贴图留在窗口内、位置不变（`content_rect` 靠 `menu_shift` 折回去）。
        assert!(placement.need_physical.x >= original.x);
        assert!(placement.need_physical.y >= original.y);
        // 只往左/上挪，且窗口顶不出屏幕。
        assert!(placement.shift_physical.x <= 0.0 && placement.shift_physical.y <= 0.0);
        assert!(outer.y + placement.shift_physical.y >= 0.0);
        assert!(outer.x + placement.shift_physical.x >= 0.0);
    }

    /// 屏幕连「贴图 + 菜单」都装不下时：菜单优先，窗口顶贴住屏幕上沿，
    /// 贴图仍在屏幕上原地不动，绝不 panic。
    #[test]
    fn menu_placement_degrades_when_the_screen_is_too_short() {
        let outer = Pos2::new(1656.0, 898.0);
        let original = Vec2::new(264.0, 144.0);
        // 比菜单还矮的屏幕：菜单自己就要 288 + 10 点，纵向一定塞不下。
        let monitor = Vec2::new(1920.0, 400.0);
        let local = Vec2::new(132.0, 72.0);
        let placement = menu_placement(local, outer, original, monitor);

        // 横向：菜单屏幕落点 `min(1788, 1710) = 1710`，窗口让到 `1710 - 132 = 1578`
        // （`shift.x = -78`）。纵向：落点 `min(970, 102) = 102`，窗口让到 `102 - 72 = 30`
        // （`shift.y = -868`）—— 这块屏幕（400 点高）连菜单（298 点）加窗口都放不下，
        // 多出来的一截由窗口/屏幕裁掉，能露多少露多少。
        assert_eq!(placement.shift_physical, Vec2::new(-78.0, -868.0));
        // 菜单屏幕落点 (1710, 102) 减窗口原点 (1578, 30)。
        assert_eq!(placement.anchor_physical, Pos2::new(132.0, 72.0));
        // `need` = max(贴图让位后的右/下沿, 菜单右/下沿)：横向 max(264 + 78, 132 + 210)
        // = 342；纵向 max(144 + 868, 72 + 298) = 1012。
        assert_eq!(placement.need_physical, Vec2::new(342.0, 1012.0));
        // 挪动量永远在 `[-outer, 0]`：既不会把贴图往右/下推，也不会推出屏幕上沿。
        assert!(placement.shift_physical.x <= 0.0 && placement.shift_physical.y <= 0.0);
        assert!(placement.shift_physical.x >= -outer.x);
        assert!(placement.shift_physical.y >= -outer.y);
        // 窗口尺寸没被缩到比原来还小。
        assert!(placement.need_physical.x >= original.x);
        assert!(placement.need_physical.y >= original.y);
        // 菜单右下沿一定落在窗口内（否则就会被窗口裁掉，且没有补偿手段）。
        assert!(
            placement.anchor_physical.x + MENU_WIDTH + MENU_PAD <= placement.need_physical.x + 0.01
        );
        assert!(
            placement.anchor_physical.y + MENU_HEIGHT + MENU_PAD <= placement.need_physical.y + 0.01
        );
        // 贴图也让得进窗口（屏幕装不下时贴图会滑出窗口，但仍不得 panic）。
        assert!(placement.need_physical.x.is_finite() && placement.need_physical.y.is_finite());
    }

    /// 屏幕放得下时，挪动量为零、贴图在屏幕上原地不动（回归：菜单一开窗口不许乱跳）。
    #[test]
    fn content_rect_stays_put_when_the_window_does_not_move() {
        let mut pinned = pin_with(|_| {});
        // 窗口 64×54 贴在屏幕左上角附近，右键点 (40, 72)。
        let outer = Pos2::new(200.0, 120.0);
        let original = Vec2::new(64.0, 54.0);
        pinned.window_size = original;
        let before = pinned.content_rect();

        let local = Vec2::new(40.0, 72.0);
        let placement = menu_placement(local, outer, original, Vec2::new(1920.0, 1080.0));
        assert_eq!(placement.shift_physical, Vec2::ZERO);
        assert_eq!(placement.need_physical, Vec2::new(250.0, 370.0));
        pinned.menu_window = original;
        pinned.menu_extra = placement.need_physical - original;
        pinned.menu_shift = placement.shift_physical;
        pinned.window_size = placement.need_physical;

        // 没挪窗口时，贴图在窗口里的位置和窗口原点都没变。
        assert!((pinned.content_rect().min - before.min).length() < 0.01);
        assert!((pinned.content_rect().size() - before.size()).length() < 0.01);
    }

    /// 挪窗口之后，`content_rect` 的补偿要把贴图**钉在屏幕上原地不动**。
    #[test]
    fn content_rect_compensates_the_window_move() {
        let mut pinned = pin_with(|_| {});

        // 故意挑「窗口比工作区还高」的屏幕，逼出非零挪动量（正常屏幕上窗口原地撑大即可）。
        let outer = Pos2::new(1656.0, 400.0);
        let original = Vec2::new(240.0, 400.0);
        pinned.window_size = original;
        let before = outer + pinned.content_rect().min.to_vec2();

        let local = Vec2::new(40.0, 72.0);
        // 竖直：needed = 400（贴图比菜单高），屏幕只有 400 → 窗口塞不下 → 得挪。
        let placement = menu_placement(local, outer, original, Vec2::new(1920.0, 400.0));
        assert!(
            placement.shift_physical.y.abs() > 0.01,
            "这个用例要逼出非零挪动量，实际={:?}",
            placement.shift_physical
        );
        pinned.menu_window = original;
        pinned.menu_extra = placement.need_physical - original;
        pinned.menu_shift = placement.shift_physical;
        pinned.window_size = placement.need_physical;

        // 贴图在窗口里挪了 `-shift`，屏幕坐标 = 窗口原点 + 窗口内位置 = outer + shift - shift。
        let content = pinned.content_rect();
        assert!(
            (content.min.x + placement.shift_physical.x).abs() < 0.01
                && (content.min.y + placement.shift_physical.y).abs() < 0.01,
            "补偿量要正好抵消窗口位移：content={content:?} shift={:?}",
            placement.shift_physical
        );
        let screen = outer + placement.shift_physical + content.min.to_vec2();
        assert!(
            (before - screen).length() < 0.01,
            "贴图在屏幕上不能动：before={before:?} screen={screen:?}"
        );
    }

    /// `Ctrl+滚轮` 调不透明度：一格 5%，两端夹住，且**不能再顺手把贴图缩放掉**。
    #[test]
    fn opacity_wheel_steps_and_clamps() {
        let mut pinned = pin_with(|_| {});

        // 往上滚 = 更不透明，但已经 100% 就停在 100%（默认 0.95，先顶到顶）
        pinned.opacity = 1.0;
        assert_eq!(pinned.opacity, 1.0);
        assert!(!pinned.adjust_opacity(120.0), "到顶之后不该再报变化");
        assert_eq!(pinned.opacity, 1.0);

        // 往下滚：一格 5%
        assert!(pinned.adjust_opacity(-120.0));
        assert!((pinned.opacity - 0.95).abs() < 1e-6, "实际={}", pinned.opacity);

        // 一路滚到底：夹在 MIN_OPACITY，而不是掉到 0（0 = 完全看不见，用户就找不回来了）
        for _ in 0..100 {
            pinned.adjust_opacity(-120.0);
        }
        assert!((pinned.opacity - MIN_OPACITY).abs() < 1e-6, "实际={}", pinned.opacity);
        assert!(!pinned.adjust_opacity(-120.0), "到底之后不该再报变化");
    }

    /// 调不透明度的滚轮事件不能同时改缩放：两个手感混在一起会很难受。
    #[test]
    fn opacity_wheel_does_not_zoom() {
        let mut pinned = pin_with(|_| {});
        pinned.opacity = 0.5;
        let before = pinned.content_size();
        assert!(pinned.adjust_opacity(-120.0));
        assert_eq!(pinned.content_size(), before);
    }

    /// 贴图边上的那圈"抓手"：角优先、边其次，中间不算。
    #[test]
    fn edge_at_finds_corners_before_edges_and_misses_the_middle() {
        let content = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 100.0));
        // 中间：没有抓手，按下是移动窗口
        assert_eq!(edge_at(content, egui::pos2(110.0, 60.0)), None);
        // 四条边
        assert_eq!(edge_at(content, egui::pos2(110.0, 11.0)), Some(PinEdge::N));
        assert_eq!(edge_at(content, egui::pos2(110.0, 109.0)), Some(PinEdge::S));
        assert_eq!(edge_at(content, egui::pos2(11.0, 60.0)), Some(PinEdge::W));
        assert_eq!(edge_at(content, egui::pos2(209.0, 60.0)), Some(PinEdge::E));
        // 四个角：两个方向同时在容差内时按角处理
        assert_eq!(edge_at(content, egui::pos2(11.0, 11.0)), Some(PinEdge::Nw));
        assert_eq!(edge_at(content, egui::pos2(209.0, 11.0)), Some(PinEdge::Ne));
        assert_eq!(edge_at(content, egui::pos2(11.0, 109.0)), Some(PinEdge::Sw));
        assert_eq!(edge_at(content, egui::pos2(209.0, 109.0)), Some(PinEdge::Se));
        // 离得太远：不算抓手（否则整张图都变成"拉边"）
        assert_eq!(edge_at(content, egui::pos2(110.0, 120.0)), None);
    }

    /// 阴影那一圈也算抓手：贴图贴在屏幕边上时，最外层像素不能只认"移动窗口"。
    #[test]
    fn edge_at_window_also_grabs_the_shadow_ring() {
        let content = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 100.0));
        let window = content.expand(8.0);
        // 窗口最外层（阴影里）也是边
        assert_eq!(edge_at_window(content, window, egui::pos2(3.0, 60.0)), Some(PinEdge::W));
        assert_eq!(edge_at_window(content, window, egui::pos2(217.0, 60.0)), Some(PinEdge::E));
        assert_eq!(edge_at_window(content, window, egui::pos2(110.0, 3.0)), Some(PinEdge::N));
        assert_eq!(edge_at_window(content, window, egui::pos2(110.0, 117.0)), Some(PinEdge::S));
        // 阴影角上就是角
        assert_eq!(edge_at_window(content, window, egui::pos2(3.0, 3.0)), Some(PinEdge::Nw));
        // 中间仍然是移动窗口
        assert_eq!(edge_at_window(content, window, egui::pos2(110.0, 60.0)), None);
        // 窗口外面不算
        assert_eq!(edge_at_window(content, window, egui::pos2(240.0, 60.0)), None);
    }

    /// 拉边 = 等比缩放，而且越拉越大、往回收会变小。
    #[test]
    fn dragging_an_edge_scales_the_pin_proportionally() {
        let content = egui::vec2(200.0, 100.0);
        // 东边往右拉 20 点 → +10%
        let wider = scale_after_drag(1.0, PinEdge::E, egui::vec2(20.0, 0.0), content);
        assert!((wider - 1.1).abs() < 1e-6, "实际={wider}");
        // 西边往左拉 20 点也是变大（方向符号要跟着边翻转）
        let from_west = scale_after_drag(1.0, PinEdge::W, egui::vec2(-20.0, 0.0), content);
        assert!((from_west - 1.1).abs() < 1e-6, "实际={from_west}");
        // 往回拉就变小
        let smaller = scale_after_drag(1.0, PinEdge::E, egui::vec2(-40.0, 0.0), content);
        assert!((smaller - 0.8).abs() < 1e-6, "实际={smaller}");
        // 角上斜着拉：取变化更大的那个轴，两个轴都往里收不会互相抵消
        let corner = scale_after_drag(1.0, PinEdge::Se, egui::vec2(40.0, 50.0), content);
        assert!((corner - 1.5).abs() < 1e-6, "实际={corner}");
        // 夹在允许范围里
        let tiny = scale_after_drag(1.0, PinEdge::E, egui::vec2(-5000.0, 0.0), content);
        assert!((tiny - MIN_SCALE).abs() < 1e-6, "实际={tiny}");
        let huge = scale_after_drag(1.0, PinEdge::E, egui::vec2(5000.0, 0.0), content);
        assert!((huge - MAX_SCALE).abs() < 1e-6, "实际={huge}");
    }
}
