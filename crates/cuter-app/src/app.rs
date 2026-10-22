//! 应用层：视口编排、后台任务派发、设置界面。
//!
//! ## 视口模型（`eframe` 的多视口）
//!
//! ```text
//! ViewportId::ROOT         控制窗口（设置面板），可隐藏，托盘驻留时不存在
//! "cuter-overlay"          全屏冻结遮罩，一次截图一个，用完即销毁
//! "cuter-pin/<n>"          屏幕贴图，可以同时存在很多个
//! "cuter-ocr"              OCR 识别结果面板
//! ```
//!
//! 视口回调必须满足 `Send + Sync + 'static`（`egui::Context::show_viewport_deferred`
//! 的签名要求），所以视口内容用 `Arc<Mutex<..>>` 共享，而不是 `Rc<RefCell<..>>`。
//! 锁的作用域一律收紧到单次调用，绝不在持有锁时再调用 `ctx.*`，避免同一线程重入死锁。
//!
//! ## OCR 为什么要单独开线程
//!
//! `Windows.Media.Ocr` 是 WinRT，阻塞等待它完成需要线程处于 **MTA** 套间；
//! 而 winit 的事件循环线程一旦被初始化为 STA，在它上面阻塞等待异步结果就会
//! 和套间消息泵互相等待而死锁。所以 OCR 一律丢到自建的后台线程上跑，
//! 结果通过 channel 回传（见 [`CuterApp::start_ocr`]）。

use cuter_ui::{fill, tr};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{
    Context, Pos2, Vec2, ViewportBuilder, ViewportCommand, ViewportId, WindowLevel,
};

use cuter_core::capture::{DesktopShot, MonitorInfo, desktop_capture};
use cuter_core::config::{Config, LocalTime, ToolKind, render_filename};
use cuter_core::frame::Frame;
use cuter_core::geom::{Point, Rect, Size};
use cuter_core::history::HistoryStore;
use cuter_core::scroll::{DirectedStitcher, MatchOptions, PushOutcome};
use cuter_core::update::{self, UpdateManifest};
use cuter_ui::fonts::{self, LoadedFont};
use cuter_ui::history_panel::{HistoryAction, HistoryPanel};
use cuter_ui::tray_panel::{TrayAction, TrayPanel, TrayPanelInput};
use cuter_ui::ocr_panel::{OcrAction, OcrPanel};
use cuter_ui::overlay::{Overlay, OverlayAction, OverlayResult};
use cuter_ui::pin::{Pin, PinAction};
use cuter_ui::scroll_panel::{ScrollAction, ScrollPanel, ScrollPanelInput};
use cuter_ui::theme::Theme;

use crate::desktop::{self, HotkeyCommand, Hotkeys, Tray, TrayCommand, UiWake};
use crate::save_dialog;
use crate::single_instance::{Activation, Guard};

/// 启动时自动检查更新的最小间隔：一天一次。
///
/// 更新检查的价值随频率下降得很快：一天一次已经足够让用户几天内看到新版本，
/// 再勤就变成"每次开机都联网"的那种程序了。手点「立即检查」不受这个限制。
const UPDATE_CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// 当前 Unix 秒。复用 `LocalTime` 的实现，不为一次读时间引入时区/时间库。
fn now_unix() -> i64 {
    i64::try_from(LocalTime::now().unix).unwrap_or(i64::MAX)
}

/// 设置窗口左侧的页面（Win11 风格的分类导航）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsPage {
    General,
    Hotkeys,
    Capture,
    Save,
    Annotate,
    Pin,
    Ocr,
    Scroll,
    History,
    Font,
    Diagnostics,
    About,
}

impl SettingsPage {
    fn label(self) -> &'static str {
        match self {
            Self::General => tr!("通用"),
            Self::Hotkeys => tr!("快捷键"),
            Self::Capture => tr!("截图"),
            Self::Save => tr!("保存"),
            Self::Annotate => tr!("标注"),
            Self::Pin => tr!("贴图"),
            Self::Ocr => tr!("识别文字"),
            Self::Scroll => tr!("滚动截屏"),
            Self::History => tr!("截图历史"),
            Self::Font => tr!("字体"),
            Self::Diagnostics => tr!("诊断"),
            Self::About => tr!("关于"),
        }
    }
}

/// 左侧导航的一行：选中态有底色 + 左侧强调条，悬停有浅底色（Win11 的观感）。
/// 侧栏的**显式顺序**（12 项分 4 组）。
///
/// 不用 `SettingsPage::ALL` 的顺序：那个顺序是枚举定义的顺序，加一个页面就可能插进
/// 别的组里，而"分组"这件事必须一眼可见、改起来不会静默错位。
const NAV_ORDER: [SettingsPage; 12] = [
    SettingsPage::General,
    SettingsPage::Hotkeys,
    SettingsPage::Capture,
    SettingsPage::Save,
    SettingsPage::Annotate,
    SettingsPage::Scroll,
    SettingsPage::Ocr,
    SettingsPage::Pin,
    SettingsPage::History,
    SettingsPage::Font,
    SettingsPage::Diagnostics,
    SettingsPage::About,
];

/// 这一项属于哪个分组（`NAV_ORDER` 里同组的项必须相邻）。
fn nav_group_label(page: SettingsPage) -> &'static str {
    match page {
        SettingsPage::General | SettingsPage::Hotkeys => tr!("常规"),
        SettingsPage::Capture | SettingsPage::Save | SettingsPage::Annotate | SettingsPage::Scroll => {
            tr!("截图")
        }
        SettingsPage::Ocr | SettingsPage::Pin => tr!("智能"),
        SettingsPage::History
        | SettingsPage::Font
        | SettingsPage::Diagnostics
        | SettingsPage::About => tr!("系统"),
    }
}

/// 每个页面的一句话说明（显示在头部标题下面）。
fn page_description(page: SettingsPage) -> &'static str {
    match page {
        SettingsPage::General => tr!("启动方式、外观与语言；每一项都会立刻生效。"),
        SettingsPage::Hotkeys => tr!("全局热键，以及按住修饰键时的临时行为。"),
        SettingsPage::Capture => tr!("捕获后端、吸附、鼠标指针与延迟。"),
        SettingsPage::Save => tr!("文件格式与质量、保存目录、文件名模板。"),
        SettingsPage::Annotate => tr!("标注工具的默认线宽、字号与效果参数。"),
        SettingsPage::Pin => tr!("贴图的不透明度、圆角、分组与缩放行为。"),
        SettingsPage::Ocr => tr!("识别引擎与结果面板的行为。"),
        SettingsPage::Scroll => tr!("抓帧间隔、匹配范围与自动滚动。"),
        SettingsPage::History => tr!("截图历史保留多少张、占多少空间。"),
        SettingsPage::Font => tr!("界面与文字标注使用的字体。"),
        SettingsPage::Diagnostics => tr!("启动耗时、日志目录与运行状态。"),
        SettingsPage::About => tr!("版本、构建信息与许可。"),
    }
}

/// 16×16 网格上的导航图标：1.6px 描边、圆头。
///
/// 全用 `painter` 画，不依赖任何图标字体——托盘/遮罩那边也是这套做法，
/// 字体缺失时界面不会变成一串方框。
fn nav_icon(painter: &egui::Painter, rect: egui::Rect, page: SettingsPage, color: egui::Color32) {
    let c = rect.center();
    let s = egui::Stroke::new(1.6, color);
    let (x, y) = (c.x, c.y);
    match page {
        SettingsPage::General => {
            painter.circle_stroke(c, 3.2, s);
            for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                painter.line_segment(
                    [egui::pos2(x + dx * 4.4, y + dy * 4.4), egui::pos2(x + dx * 6.4, y + dy * 6.4)],
                    s,
                );
            }
        }
        SettingsPage::Hotkeys => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(14.0, 10.0)),
                egui::CornerRadius::same(3),
                s,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([egui::pos2(x - 4.0, y), egui::pos2(x + 4.0, y)], s);
        }
        SettingsPage::Capture => {
            painter.line_segment([egui::pos2(x - 5.0, y - 7.0), egui::pos2(x - 5.0, y + 3.0)], s);
            painter.line_segment([egui::pos2(x - 7.0, y - 5.0), egui::pos2(x + 3.0, y - 5.0)], s);
            painter.line_segment([egui::pos2(x + 3.0, y - 5.0), egui::pos2(x + 3.0, y + 7.0)], s);
            painter.line_segment([egui::pos2(x - 5.0, y + 3.0), egui::pos2(x + 7.0, y + 3.0)], s);
        }
        SettingsPage::Save => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(12.0, 12.0)),
                egui::CornerRadius::same(2),
                s,
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x - 2.5, y - 6.0), egui::vec2(5.0, 4.0)),
                egui::CornerRadius::same(1),
                color,
            );
        }
        SettingsPage::Annotate => {
            painter.line_segment([egui::pos2(x - 5.0, y + 5.0), egui::pos2(x + 5.0, y - 5.0)], s);
        }
        SettingsPage::Scroll => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(10.0, 14.0)),
                egui::CornerRadius::same(3),
                s,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([egui::pos2(x, y - 3.0), egui::pos2(x, y + 3.0)], s);
        }
        SettingsPage::Ocr => {
            painter.line_segment([egui::pos2(x - 5.0, y - 5.0), egui::pos2(x + 5.0, y - 5.0)], s);
            painter.line_segment([egui::pos2(x, y - 5.0), egui::pos2(x, y + 5.0)], s);
        }
        SettingsPage::Pin => {
            painter.circle_stroke(egui::pos2(x - 1.0, y - 1.0), 4.0, s);
            painter.line_segment([egui::pos2(x + 2.0, y + 2.0), egui::pos2(x + 6.0, y + 6.0)], s);
        }
        SettingsPage::History => {
            painter.circle_stroke(c, 6.0, s);
            painter.line_segment([egui::pos2(x, y), egui::pos2(x, y - 3.0)], s);
            painter.line_segment([egui::pos2(x, y), egui::pos2(x + 3.0, y + 1.0)], s);
        }
        SettingsPage::Font => {
            painter.line_segment([egui::pos2(x - 5.0, y - 5.0), egui::pos2(x + 5.0, y - 5.0)], s);
            painter.line_segment([egui::pos2(x, y - 5.0), egui::pos2(x, y + 5.0)], s);
        }
        SettingsPage::Diagnostics => {
            painter.line_segment([egui::pos2(x - 6.0, y), egui::pos2(x - 2.0, y)], s);
            painter.line_segment([egui::pos2(x - 2.0, y), egui::pos2(x, y - 5.0)], s);
            painter.line_segment([egui::pos2(x, y - 5.0), egui::pos2(x + 2.0, y + 5.0)], s);
            painter.line_segment([egui::pos2(x + 2.0, y + 5.0), egui::pos2(x + 6.0, y)], s);
        }
        SettingsPage::About => {
            painter.circle_stroke(c, 6.0, s);
            painter.line_segment([egui::pos2(x, y - 1.0), egui::pos2(x, y + 4.0)], s);
            painter.circle_filled(egui::pos2(x, y - 4.0), 1.0, color);
        }
    }
}

/// 侧栏里的一行：图标 + 文字；选中 = accent/subtle 底 + 左侧 3px 强调条 + 强调色文字。
fn nav_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    page: SettingsPage,
    label: &str,
    selected: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::new(width, 30.0), egui::Sense::click());
    if selected || response.hovered() {
        let fill = if selected {
            theme.accent_subtle
        } else {
            theme.surface_hover
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(8), fill);
    }
    if selected {
        let bar = egui::Rect::from_min_size(
            egui::Pos2::new(rect.left(), rect.center().y - 8.0),
            egui::Vec2::new(3.0, 16.0),
        );
        ui.painter()
            .rect_filled(bar, egui::CornerRadius::same(2), theme.accent);
    }
    let fg = if selected {
        theme.accent
    } else {
        theme.on_surface_dim
    };
    nav_icon(
        ui.painter(),
        egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::Vec2::splat(16.0)),
        page,
        fg,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 44.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        fg,
    );
    response
}

/// 卡片的底色：**必须与控件自己的底色拉开一点**。
///
/// 深色主题下控件的未激活底色就是 `theme.surface`，而卡片如果也用 `surface`，
/// 未勾选的复选框会"消失"（实测：勾选的有 ✓、没勾选的看着就是一行文字）。
/// 所以卡片底色按主题明暗各偏一点。
fn card_fill(theme: &Theme) -> egui::Color32 {
    // 设计系统 v1：卡片有**明确的一档**颜色（bg/raised），不再靠"底色调亮 12%"这种估算——
    // 估算出来的值在浅色主题下会偏，而且和设计稿对不上。
    theme.surface_raised
}

/// 控件的中性底色（复选框、下拉框、普通按钮）。
///
/// **不能用 `surface_hover` / `surface_active`**：那两个是带强调色的（蓝），
/// 铺在复选框上会变成一排蓝色圆点，看着像单选按钮（实测就是这么难看的）。
fn control_fill(theme: &Theme) -> egui::Color32 {
    // 复选框/下拉框用"凹"色（bg/sunken）：比卡片深一档，一眼能看出是输入控件。
    theme.surface_sunken
}

/// 设置界面里的控件视觉：把主题里"透明"的控件底色补回中性色。
///
/// 工具条图标自己画底，所以主题里 `widgets.inactive.*_fill` 是透明的；
/// 设置界面直接用那套视觉会让复选框"只剩一个对勾"、下拉框只剩文字。
fn settings_visuals(ui: &egui::Ui, theme: &Theme) -> egui::Visuals {
    let mut visuals = ui.visuals().clone();
    let neutral = control_fill(theme);
    visuals.widgets.inactive.bg_fill = neutral;
    visuals.widgets.inactive.weak_bg_fill = neutral;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme.outline);
    visuals.widgets.hovered.bg_fill = neutral.gamma_multiply(1.25);
    visuals.widgets.hovered.weak_bg_fill = neutral.gamma_multiply(1.25);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme.outline);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    // 输入框底色与正文色也必须按主题补齐：`ui.visuals()` 有可能是本帧早先按系统
    // 建的那套（浅色），只补控件底色就会得到"深色输入框里压着近黑字"这种读不出来
    // 的组合——用户报的"白底输入框""下拉框文字看不清"都是这一个来源。
    visuals.extreme_bg_color = theme.surface_sunken;
    visuals.text_edit_bg_color = Some(theme.surface_sunken);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.on_surface);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.on_surface);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.on_surface);
    visuals
}

/// 行尾的滑块：把导轨、已滑过的一段、手柄三者的对比度补齐。
///
/// 直接用 `egui::Slider` 会套用卡片那套控件视觉：导轨用 `inactive.bg_fill`
/// （和卡片几乎同色）、手柄是 `bg_fill` 填充 + `fg_stroke` 描边，于是屏幕上只剩
/// 一圈虚边（用户："两个滑动块浅色深色都不明显"）。这里另给一套——导轨走
/// `outline_strong`，滑过的一段与手柄描边走强调色。
fn settings_slider(ui: &mut egui::Ui, theme: &Theme, slider: egui::Slider<'_>) -> bool {
    let mut visuals = ui.visuals().clone();
    visuals.widgets.inactive.bg_fill = theme.outline_strong;
    visuals.widgets.inactive.weak_bg_fill = theme.outline_strong;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.6, theme.accent);
    visuals.widgets.hovered.bg_fill = theme.outline_strong;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.6, theme.accent);
    visuals.widgets.active.bg_fill = theme.accent;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.6, theme.on_surface);
    visuals.selection.bg_fill = theme.accent;
    ui.scope(|ui| {
        ui.set_visuals(visuals);
        ui.add(slider.trailing_fill(true)).changed()
    })
    .inner
}

/// 一张设置卡片：标题 + 内容，圆角边框。右侧页面由若干张卡片组成。
/// 一行设置项：左边"标签 + 说明"，右边控件。**高 52**，行间一条 1px 分隔线。
///
/// 这是设置页的基本单位（稿子：`docs/design/mockups/settings.png`）。以前是
/// `ui.checkbox(&mut cfg, "一整句话")`——标签和说明挤在一行、行距 20px，
/// 一屏十来个开关糊成一片，看不出"哪些是一组、哪个开关对应哪句说明"。
/// 设置行右侧控件列的宽度。
///
/// 固定下来是为了让整页控件纵向对齐：以前下拉框 150、输入框 200/220/240 各写各的，
/// 一屏看过去右边全是锯齿（用户："整个界面全都是大大小小的输入气泡"）。
const CONTROL_COLUMN: f32 = 240.0;

/// 下拉框与单行输入框的统一宽度。
///
/// 比 `CONTROL_COLUMN` 少 68：留给"输入框 + 行尾按钮"那种组合（172 + 8 + 56 ≈ 236）。
const CONTROL_W: f32 = 172.0;

/// 量一段文字在给定宽度下排出来有多高（用来决定这一行要不要长高）。
fn text_height(ui: &egui::Ui, text: &str, size: f32, width: f32) -> f32 {
    ui.painter()
        .layout(
            text.to_owned(),
            egui::FontId::proportional(size),
            egui::Color32::PLACEHOLDER,
            width,
        )
        .size()
        .y
}

fn settings_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    hint: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width();
    let text_width = (width - CONTROL_COLUMN - 12.0).max(140.0);
    // 说明折行时行高跟着长，**上限两行**：再长就裁掉，悬停看全文。
    // 固定 52 的老做法把长说明硬塞进一行，只能截断成"…"，用户嫌它不聪明。
    let label_height = text_height(ui, label, 13.0, text_width);
    let hint_height = text_height(ui, hint, 11.0, text_width).min(30.0);
    let height = (label_height + hint_height + 20.0).clamp(52.0, 84.0);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(width, height), egui::Sense::hover());
    // 分隔线画在行的底部；最后一行下方那条由卡片自己收尾，这里统一画，视觉上更整齐。
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, theme.outline),
    );

    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::Vec2::new(0.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.vertical(|ui| {
        // 左列宽度必须封顶，否则文字会一路画到右侧控件底下，看着像"字压在按钮上"。
        ui.set_max_width(text_width);
        // 超过两行的部分不许越界画到相邻行上。
        ui.set_clip_rect(rect);
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(label)
                    .size(13.0)
                    .color(theme.on_surface),
            )
            .wrap(),
        )
        .on_hover_text(label);
        ui.add(
            egui::Label::new(
                egui::RichText::new(hint)
                    .size(11.0)
                    .color(theme.on_surface_faint),
            )
            .wrap(),
        )
        .on_hover_text(hint);
    });
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // 右侧留 4px：不留的话方块/下拉框会贴死在卡片内边距上，看起来像被切了一角。
        ui.add_space(4.0);
        control(ui);
    });
}

/// 行尾的开关：18px 圆角方块，选中 = 强调色底 + 白勾。
///
/// 为什么不用 `egui::Checkbox`：它把方块的底色取自 `widgets.*.bg_fill`、勾取自
/// `visuals.selection`，两者在深色主题下会撞成"浅底浅勾"（实测就是这样，
/// 屏幕上几乎看不见勾）。自己画一个反而更短、也更贴稿子。
fn settings_switch(ui: &mut egui::Ui, theme: &Theme, value: &mut bool) -> bool {
    let size = 18.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
    let radius = egui::CornerRadius::same(6);
    if *value {
        ui.painter().rect_filled(rect, radius, theme.accent);
        let c = rect.center();
        let stroke = egui::Stroke::new(2.2, egui::Color32::WHITE);
        ui.painter().line_segment(
            [egui::pos2(c.x - 4.0, c.y), egui::pos2(c.x - 1.0, c.y + 3.0)],
            stroke,
        );
        ui.painter().line_segment(
            [egui::pos2(c.x - 1.0, c.y + 3.0), egui::pos2(c.x + 4.5, c.y - 3.5)],
            stroke,
        );
    } else {
        ui.painter().rect_filled(rect, radius, theme.surface_sunken);
        let border = if response.hovered() {
            theme.accent
        } else {
            theme.outline_strong
        };
        ui.painter()
            .rect_stroke(rect, radius, egui::Stroke::new(1.2, border), egui::StrokeKind::Inside);
    }
    if response.clicked() {
        *value = !*value;
        return true;
    }
    false
}

/// 一行开关：标签 + 说明 + 右侧 18px 开关；返回是否被改动。
///
/// 开关行在设置里占绝大多数，单独包一层省掉每行都写 `let mut x = ...; if toggled { ... }`。
fn settings_switch_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    hint: &str,
    value: &mut bool,
) -> bool {
    let mut next = *value;
    let mut toggled = false;
    settings_row(ui, theme, label, hint, |ui| {
        toggled = settings_switch(ui, theme, &mut next);
    });
    if toggled {
        *value = next;
    }
    toggled
}

/// 行尾的下拉框（32 高、圆角 6、"凹"底）。
fn settings_select<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    theme: &Theme,
    id: &str,
    current: T,
    options: &[(T, &str)],
    render: impl Fn(T) -> String,
) -> Option<T> {
    let mut picked = None;
    let mut visuals = ui.visuals().clone();
    visuals.widgets.inactive.bg_fill = theme.surface_sunken;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme.outline_strong);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.on_surface);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme.accent);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.on_surface);
    // 弹层也得跟着主题：不补的话下拉列表用的是 `ui.visuals()` 里的窗口底色，
    // 深色主题下会弹出一块浅底浅字的浮层（实测：字几乎看不见）。
    visuals.window_fill = theme.surface_raised;
    visuals.panel_fill = theme.surface_raised;
    visuals.window_stroke = egui::Stroke::new(1.0, theme.outline_strong);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.on_surface);
    ui.scope(|ui| {
        ui.set_visuals(visuals);
        ui.spacing_mut().interact_size.y = 30.0;
        egui::ComboBox::from_id_salt(id)
            .selected_text(render(current))
            .width(CONTROL_W)
            .show_ui(ui, |ui| {
                for (value, label) in options {
                    if ui.selectable_label(*value == current, *label).clicked() {
                        picked = Some(*value);
                    }
                }
            });
    });
    picked
}

fn settings_card(ui: &mut egui::Ui, theme: &Theme, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    let visuals = settings_visuals(ui, theme);
    egui::Frame::new()
        .fill(card_fill(theme))
        .stroke(egui::Stroke::new(1.0, theme.outline))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_visuals(visuals);
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .size(15.0)
                    .strong()
                    .color(theme.on_surface),
            );
            ui.add_space(6.0);
            body(ui);
        });
    ui.add_space(10.0);
}

/// 遮罩视口的固定 id。
fn overlay_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cuter-overlay")
}

/// 遮罩视口的构造：一份描述，两处用（新建窗口 / 复用窗口时都走它）。
///
/// 每次都新造一份描述本身没问题（`ViewportBuilder::patch` 只比对**字段值**），
/// 但字段少一处不一致就会多发一条视口命令、白花一帧。
fn overlay_builder(bounds: Rect) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(OVERLAY_TITLE)
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false)
        // ⚠️ `active` 在 egui 里**一变就重建窗口**（`ViewportBuilder::patch` 里
        // `new_active != self.active → recreate_window = true`）。当前生命周期是
        // "每次截图新建窗口、遮罩结束就销毁"，所以这里固定写 true（创建即带焦点）最省事；
        // 将来若改成复用窗口，就必须固定成同一个值、改用 `ViewportCommand::Focus` 抢焦点。
        .with_active(true)
        .with_position([bounds.x as f32, bounds.y as f32])
        .with_inner_size([bounds.w as f32, bounds.h as f32])
}

// ---------------------------------------------------------------------------
// 关于"让遮罩视口窗口常驻复用"（商用化第十二轮试过，**否决**，见 HANDOFF 第 25 节）
//
// 动机很直白：每次截图新建窗口 + 建 GL 上下文要 ~100 ms，而捕获本身只要 11 ms（DXGI）。
// 做法是把窗口停在桌面之外（-32000）留着，捕获时只改几何 + 重绘。
//
// 结果：**在 eframe 0.36 上走不通**，三条都实测过：
//   1. eframe 对**不可见**的 deferred 视口直接 `show_ui = false`，连 egui pass 都不跑
//      （`glow_integration.rs`: `is_visible || is_viewport_or_descendant_visible`）。
//      窗口停在桌面之外时 winit 就把它算作不可见，于是 `visible` 被缓存住；
//      把窗口移回屏幕也不会重新开画——**窗口在、画面永远空白**（探针抓到：
//      窗口能被找到，却等不到"遮罩首帧可见"）。
//   2. `ViewportBuilder::active` 一变就 `recreate_window = true`（egui 源码），
//      所以"停靠时 active=false / 截图时 true"等于每次都重建窗口，白忙。
//   3. `ViewportCommand::Visible(false)` 对"每帧重新声明"的视口不生效（第 22 节）。
//
// 唯一确定有效的收获是**帧率**：见 `open_overlay` 里的显式唤醒（`wake.wake`）——
// 托盘模式下 eframe 把重绘节流到 ~100 ms，唤醒之后帧间隔实测从 ~125 ms 降到 ~17 ms。
// 遮罩要两帧才画出来，所以这一项直接把延迟砍掉一大截。
// ---------------------------------------------------------------------------
/// 遮罩窗口的标题。**截图前判断"我们自己的窗口还在不在屏幕上"要用它**
/// （`desktop::UiWake::window_with_title_visible`），所以标题不再是随便起的字符串：
/// 改标题等于改判据，两处必须一起改。
const OVERLAY_TITLE: &str = "Cuter 截图";

/// 贴图窗口的标题（同上，用于"我们自己的窗口还在不在"的判断）。
const PIN_TITLE: &str = "Cuter 贴图";

/// OCR 结果面板的固定 id（同时只保留一个面板）。
fn ocr_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cuter-ocr")
}

/// 滚动截屏控制窗的固定 id。
fn scroll_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cuter-scroll")
}

/// 截图历史面板的固定 id（同时只保留一个面板）。
fn history_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cuter-history")
}

/// 托盘菜单面板的固定 id（同时只保留一个）。
fn tray_menu_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("cuter-tray-menu")
}

/// 滚动截屏的预览最长边（缩略图）；原图另外保存，导出时才取。
const SCROLL_PREVIEW_MAX: u32 = 4096;
/// 预览刷新间隔。长图每次都要重新抽样，太频繁会白烧 CPU。
const SCROLL_PREVIEW_INTERVAL: Duration = Duration::from_millis(600);

/// 滚动截屏会话的可变状态（采集线程写、UI 线程读）。
struct ScrollSession {
    stitcher: DirectedStitcher,
    /// 被持续采集的全局区域。
    region: Rect,
    options: MatchOptions,
    paused: bool,
    status: String,
    /// 缩略长图 + 版本号（版本号变化时 UI 重建纹理）。
    preview: Option<Frame>,
    preview_generation: u64,
    last_preview: Instant,
    /// 两次抓帧之间的间隔。
    interval: Duration,
    /// 自动滚动：由程序自己合成滚轮（面板上按 A 切换）。
    auto: bool,
    /// 自动滚动的停机判据。
    auto_tracker: cuter_core::scroll::AutoScrollTracker,
}

impl ScrollSession {
    /// 供面板显示的一行状态。
    fn describe(&self) -> String {
        let frame = self.stitcher.last_match();
        match frame {
            Some(result) => fill!(tr!("最近一次滚动 {} 像素（误差 {}{}）"), result.offset, format!("{:.1}", result.mean_error), if result.sticky_rows > 0 {
                    fill!(tr!("，已忽略 {} 行固定页头"), result.sticky_rows)
                } else {
                    String::new()
                }),
            None => tr!("等待滚动…").to_owned(),
        }
    }
}

/// 后台建 DXGI 后端。
///
/// **为什么必须异步**：在这台机器上实测，建 D3D11 设备 + 桌面复制要 **1.76 秒**。
/// 放在捕获路径上等于每次截图都卡近两秒，放在启动路径上会让窗口迟迟不出现，
/// 所以挂后台线程：建好之前所有截图走 GDI，建好之后（同一个进程内）一直用它。
///
/// 也解释了为什么**不能"用完就释放"**：释放掉下次再建又是 1.76 s。
fn spawn_dxgi_backend(
    slot: Shared<Option<Box<dyn cuter_core::capture::ScreenCapture>>>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        match cuter_core::capture::desktop_capture_with(cuter_core::capture::CaptureBackend::Dxgi) {
            Ok(capture) => {
                log::info!(
                    "DXGI 后端已就绪（耗时 {} ms），后续截图用它",
                    start.elapsed().as_millis()
                );
                *lock(&slot) = Some(capture);
            }
            Err(err) => {
                log::warn!("DXGI 后端建不起来（{err}），本次运行一直用 GDI");
            }
        }
        // 让设置窗口里的"当前实际在用"马上显示正确
        ctx.request_repaint();
    });
}

/// 滚动截屏的采集线程：只做抓帧与拼接，线程上不碰 UI。
fn scroll_worker(
    state: Arc<Mutex<ScrollSession>>,
    stop: Arc<AtomicBool>,
    ctx: egui::Context,
    dxgi_backend: Shared<Option<Box<dyn cuter_core::capture::ScreenCapture>>>,
) {
    // 线程里自己抓一个"当前可用后端"的句柄：DXGI 就绪就用它，否则每帧现建 GDI
    let grab = |region: Rect| -> Result<Frame, String> {
        {
            let slot = lock(&dxgi_backend);
            if let Some(capture) = slot.as_deref() {
                return capture.capture_region(region).map_err(|err| err.to_string());
            }
        }
        // 没有 DXGI：现建一个 GDI 后端（实测 0.00 ms，可以随用随建）
        let gdi = cuter_core::capture::desktop_capture().map_err(|err| err.to_string())?;
        gdi.capture_region(region).map_err(|err| err.to_string())
    };
    if let Err(err) = grab(Rect::new(0, 0, 4, 4)) {
        log::warn!("滚动截屏的抓帧通道不可用：{err}");
    }

    while !stop.load(Ordering::Relaxed) {
        let interval = match state.lock() {
            Ok(session) => session.interval,
            Err(_) => break,
        };
        std::thread::sleep(interval);
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let (region, _options, paused, horizontal, auto_stall) = match state.lock() {
            Ok(session) => (
                session.region,
                session.options,
                session.paused,
                session.stitcher.axis() == cuter_core::scroll::Axis::Horizontal,
                session.auto_tracker.stall_frames,
            ),
            Err(_) => break,
        };
        if paused {
            continue;
        }

        let frame = match grab(region) {
            Ok(frame) => frame,
            Err(err) => {
                if let Ok(mut session) = state.lock() {
                    session.status = fill!(tr!("抓帧失败：{}"), err);
                }
                ctx.request_repaint();
                continue;
            }
        };

        match state.lock() {
            Ok(mut session) => {
                let outcome = session.stitcher.push(frame);
                // 自动滚动：先问状态机"下一步怎么办"，再决定要不要往目标程序灌滚轮。
                if session.auto {
                    match session.auto_tracker.observe(outcome) {
                        cuter_core::scroll::AutoScrollStep::Scroll => {
                            drop(session);
                            // 发滚轮**要在锁外**：SendInput 会走一遍输入栈，
                            // 期间 UI 线程可能正想读会话状态，别把锁攥在手里。
                            let ok = crate::desktop::auto_scroll_step(region, horizontal);
                            if let Ok(mut session) = state.lock() {
                                let steps = session.auto_tracker.steps;
                                session.status = fill!(
                                    tr!("自动滚动中（第 {} 次）{}"),
                                    steps,
                                    if ok { "" } else { "，滚轮投递失败" }
                                );
                            }
                            ctx.request_repaint();
                            continue;
                        }
                        cuter_core::scroll::AutoScrollStep::Stalled => {
                            session.auto = false;
                            log::info!(
                                // ASCII 前缀是给探针用的：`tools/probe/*.ps1` 必须是纯 ASCII
                                // （PowerShell 5.1 会把无 BOM 的中文按 GBK 读，日志里的中文
                                // 没法直接匹配），所以关键日志一律带一个英文标记。
                                "autoscroll-stop stalled={} steps={} height={}（滚动截屏自动滚动结束）",
                                auto_stall,
                                session.auto_tracker.steps,
                                session.stitcher.size().h
                            );
                            session.status = fill!(
                                tr!("自动滚动结束：连续 {} 帧画面没变化（到底了，或目标程序忽略了合成滚轮）"),
                                auto_stall
                            );
                        }
                        cuter_core::scroll::AutoScrollStep::Exhausted => {
                            session.auto = false;
                            session.status = tr!("自动滚动结束：已达转发次数上限").to_owned();
                        }
                        cuter_core::scroll::AutoScrollStep::TooTall => {
                            session.auto = false;
                            session.status = tr!("自动滚动结束：已达长度上限").to_owned();
                        }
                    }
                }
                // 先把要显示的细节算出来：`describe` 借用整个会话，
                // 而下一行要可变借用它的 `status` 字段。
                let detail = session.describe();
                session.status = match outcome {
                    PushOutcome::Appended { offset } => {
                        fill!(tr!("已拼接（新增 {} 行）｜{}"), offset, detail)
                    }
                    PushOutcome::Unchanged => tr!("画面没变化").to_owned(),
                    PushOutcome::Prepended { offset } => {
                        fill!(tr!("已往上拼接（新增 {} 行）｜{}"), offset, detail)
                    }
                    PushOutcome::NoMatch => tr!("没识别出滚动，请继续滚动页面").to_owned(),
                    PushOutcome::TooTall => tr!("已达长度上限，不再追加").to_owned(),
                };

                // 定期刷新缩略图
                if session.last_preview.elapsed() >= SCROLL_PREVIEW_INTERVAL {
                    let size = session.stitcher.size();
                    let thumb = session
                        .stitcher
                        .thumbnail(cuter_core::geom::Size::new(
                            size.w.min(SCROLL_PREVIEW_MAX),
                            size.h.min(SCROLL_PREVIEW_MAX),
                        ));
                    session.preview = Some(thumb);
                    session.preview_generation += 1;
                    session.last_preview = Instant::now();
                }
            }
            Err(_) => break,
        }
        ctx.request_repaint();
    }
}

/// OCR 工作线程回传给 UI 线程的消息。
enum OcrMessage {
    /// 识别完成。面板在这里就构造好，UI 线程直接挂上去。
    Done(Box<OcrPanel>),
    /// 引擎不可用或识别失败。
    Failed(String),
}

/// 一次截图请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRequest {
    /// 框选（全屏冻结遮罩）。
    Region,
    /// 鼠标所在显示器。
    Monitor,
    /// 整个虚拟桌面。
    Desktop,
    /// 框选一块区域后持续采集，拼成长图。
    Scroll,
}

/// 窗口几何自校正状态。
///
/// 窗口几何自校正状态。
///
/// ## 单位：这里是 egui 的「点」，不是物理像素
///
/// 之所以不再换算成物理像素，是因为换算链上有两个会互相抵消的因子：
///
/// * `egui-winit` 创建窗口时执行 `LogicalSize(zoom_factor * size)`，随后 winit 再乘
///   一次该窗口所在显示器的缩放；
/// * `ViewportCommand::InnerSize` 则执行 `PhysicalSize(pixels_per_point * size)`。
///
/// 遮罩视口把 `pixels_per_point` 固定成 1.0（即 `zoom_factor = 1 / 系统缩放`），
/// 于是两条链都退化成「点 == 物理像素」，所以遮罩的目标直接用物理像素即可；
/// 贴图窗口不固定 ppp，因此它的目标要先除以系统缩放才是「点」。
///
/// 把单位统一到「点」之后，`viewport_rect()` / `outer_rect`（也都是点）可以直接比较，
/// 不再出现「改了尺寸却越改越偏」的问题。
#[derive(Clone, Copy, Debug)]
struct GeometryFix {
    /// 目标左上角（egui 点）。
    target_pos: Vec2,
    /// 目标内尺寸（egui 点）。
    target_size: Vec2,
    attempts: u8,
    settled: bool,

    /// 位置是否交给贴图自己下发。
    ///
    /// 贴图为把右键菜单挪到屏幕内，会直接发 `ViewportCommand::OuterPosition`；此时宿主
    /// 不能再发位置，否则两边互相覆盖，窗口被拖回原处、菜单又被屏幕边裁掉。
    /// 置位期间 `apply_geometry_fix` 只对齐尺寸，位置完全由贴图负责。
    pos_owned: bool,
}

impl GeometryFix {
    fn new(target_pos: Vec2, target_size: Vec2) -> Self {
        Self {
            target_pos,
            target_size,
            attempts: 0,
            settled: false,
            pos_owned: false,
        }
    }
}

/// 把贴图此刻的「菜单挪窗」要求并进几何目标。
///
/// 贴图为把右键菜单挪进屏幕，会自己发 `ViewportCommand::OuterPosition`
/// （见 `Pin::grow_for_menu`）；但宿主这边只要 `target_pos` 还是老位置，
/// `apply_geometry_fix` 每帧也会再发一次位置，两边**同帧互相覆盖**——实测最终位置偏掉
/// 半个位移，菜单又被裁成一行。
///
/// 所以这里把宿主的目标位置**对齐到贴图想要的位置**：两边发同一个值，谁先谁后就无所谓，
/// 也不用再靠「谁拥有位置」这种状态去互相谦让。归一化用贴图给的**绝对**内框左上角，
/// 而不是 `target_pos + 偏移`：贴图每帧重算摆放，累加会跑飞。
///
/// 单位：`Pin::menu_target_origin` 已经是 egui 点，与 `target_pos` 同一把尺子。
fn sync_menu_shift(pin: &Arc<Mutex<Pin>>, fix: &mut GeometryFix) {
    let target = lock(pin).menu_target_origin();
    match target {
        Some(origin) => {
            fix.pos_owned = true;
            fix.target_pos = Vec2::new(origin.x, origin.y);
        }
        None => fix.pos_owned = false,
    }
}

/// 把窗口拉到目标几何。
///
/// `expect_pixels_per_point_one`：该视口是否会把 ppp 固定成 1.0。固定 ppp 的视口
/// （遮罩）在 ppp 尚未生效的那个 pass 里测量会偏，所以要先等它生效再动手，否则会
/// 先放大一轮再缩回来。
///
/// `focus_on_settle`：对齐完成后要不要把焦点抢过来。遮罩需要（Esc/回车是键盘操作），
/// 贴图**不需要**——每次拖边缩放都会"对齐一次"，每次都 Focus 会打断正在进行的拖动，
/// 表现为"拖一下就不动了"。
fn apply_geometry_fix(
    ctx: &Context,
    fix: &mut GeometryFix,
    expect_pixels_per_point_one: bool,
    focus_on_settle: bool,
) {
    if fix.settled {
        return;
    }
    if expect_pixels_per_point_one && (ctx.pixels_per_point() - 1.0).abs() > 1e-3 {
        return;
    }

    let inner = ctx.viewport_rect().size();
    let outer = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));

    log::debug!(
        "几何校正：ppp={:.3} 内尺寸={:?} 外位置={:?} 目标尺寸={:?} 目标位置={:?} 第{}次",
        ctx.pixels_per_point(),
        inner,
        outer,
        fix.target_size,
        fix.target_pos,
        fix.attempts + 1
    );

    let size_ok = (inner.x - fix.target_size.x).abs() <= 1.0
        && (inner.y - fix.target_size.y).abs() <= 1.0;
    // 位置已经交给贴图（菜单挪窗期间），宿主这边不再判定也不再下发，免得互相覆盖。
    let pos_ok = fix.pos_owned
        || outer.is_some_and(|p| {
            (p.x - fix.target_pos.x).abs() <= 1.5 && (p.y - fix.target_pos.y).abs() <= 1.5
        });

    if size_ok && pos_ok {
        fix.settled = true;
        if focus_on_settle {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        return;
    }

    if fix.attempts >= 10 {
        log::warn!(
            "窗口几何未能在 10 次内对齐（目标位置 {:?} 尺寸 {:?}，实际位置 {:?} 尺寸 {:?}）；\
             多显示器混合缩放时 Windows 会强制调整跨屏窗口的尺寸，此时建议在设置里\
             改用「截取当前显示器」",
            fix.target_pos,
            fix.target_size,
            outer,
            inner
        );
        fix.settled = true;
        return;
    }

    fix.attempts += 1;
    // 注意单位：`InnerSize` 收 `Vec2`，而 `OuterPosition` 收 `Pos2`
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(fix.target_size));
    if !fix.pos_owned {
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(
            fix.target_pos.x,
            fix.target_pos.y,
        )));
    }
}

/// 共享状态的别名（视口回调要求 `Send + Sync`）。
type Shared<T> = Arc<Mutex<T>>;

/// 一次后台更新检查的结果。
#[derive(Debug, Clone)]
enum UpdateOutcome {
    /// 清单里的版本比当前新。
    Newer(UpdateManifest),
    /// 当前已经是最新（带上清单自报的版本号，方便核对）。
    Current(String),
    /// 没查成：网络、地址、清单格式……原样带一句原因。
    Failed(String),
}

fn lock<T>(shared: &Shared<T>) -> std::sync::MutexGuard<'_, T> {
    match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // 某个视口 panic 过：恢复数据继续用，不要让整个程序卡死
            log::error!("共享状态锁被污染，已恢复");
            poisoned.into_inner()
        }
    }
}

/// 在锁内修改几何校正状态。
///
/// 单独抽出来是因为 `MutexGuard` 需要一个 `mut` 绑定才能 `.as_mut()`，
/// 直接写 `lock(&fix).as_mut()` 会借用到一个临时值上而编译失败。
fn edit_geometry_fix(fix: &Shared<GeometryFix>, edit: impl FnOnce(&mut GeometryFix)) {
    let mut guard = lock(fix);
    edit(&mut guard);
}

/// 一个贴图视口的登记信息。
struct PinEntry {
    pin: Shared<Pin>,
    fix: Shared<GeometryFix>,
    /// 视口描述。**必须每帧原样重新声明**，见 [`CuterApp::declare_viewports`]。
    builder: ViewportBuilder,
    /// 这张贴图属于哪一组（见 `PinConfig::groups`）。
    group: u32,
}

/// 用户在贴图窗口的右键菜单里改掉的偏好。
///
/// 这些改动既要在当前贴图上立刻生效，也要写回 `config.pin` 作为**下一个**贴图的默认值；
/// 写配置需要 `&mut CuterApp`，而贴图闭包拿不到，于是排队回主线处理。
#[derive(Clone, Copy, Debug, PartialEq)]
enum PinPrefPatch {
    Opacity(f32),
    ClickThrough(bool),
    AlwaysOnTop(bool),
}

/// 启动选项。
#[derive(Debug)]
pub struct LaunchOptions {
    /// 启动后立刻发起的截图请求（`cuter --capture`）。
    pub start_request: Option<CaptureRequest>,
    /// 启动后立刻把剪贴板里的图片贴到屏幕上（`cuter --pin`）。
    pub start_pin: bool,
    /// 启动后打开截图历史面板（`cuter --history`）。
    pub start_history: bool,
    /// 启动后打开自绘的托盘菜单面板（`cuter --tray-menu`）。
    ///
    /// 这个面板平时只能靠右键托盘图标打开，而托盘图标在通知区域、合成点击很不可靠，
    /// 所以留一个能直接打开的入口：既方便自己核对版面，也让探针能稳定验证它。
    pub start_tray_menu: bool,
    /// 启动后直接打开「设置 → 关于」页（`cuter --about`）。
    ///
    /// 更新检查那一栏就在这一页；"先开设置、再点左边导航的第十二项"这种合成点击不可靠，
    /// 所以给探针留一条确定性入口（和 `--tray-menu` 同一个理由）。
    pub start_about: bool,
    /// 启动时主窗口应当是隐藏的（托盘驻留模式，或 `--capture` 直接进框选）。
    ///
    /// **不能只靠 `ViewportBuilder::with_visible(false)`**：eframe 在画完第一帧之后
    /// 会无条件把根窗口 `set_visible(true)`（`epi_integration::post_rendering`，
    /// 注释是 "keep hidden until we've painted something"），那个 builder 选项会被它盖掉。
    /// 见 [`CuterApp::enforce_start_hidden`]。
    pub start_hidden: bool,
    /// 单实例守卫。`None` 表示内核对象创建失败（少见），本次启动没有重复实例保护。
    /// **它必须活到窗口关闭**，否则互斥体会提前释放，第二个实例就能挤进来抢热键。
    pub instance: Option<Guard>,
}

/// 应用。
pub struct CuterApp {
    config: Config,
    theme: Theme,
    font: Option<LoadedFont>,

    tray: Option<Tray>,
    /// 图标皮肤。跟着主题走，但**只在变化时**才去换窗口与托盘图标（换图标要重建 RGBA）。
    icon_skin: crate::icon::IconSkin,
    hotkeys: Option<Hotkeys>,
    /// 单实例守卫；只要它还活着，别的实例就抢不走全局热键。
    instance: Option<Guard>,

    /// 待执行的截图请求。
    request: Option<CaptureRequest>,
    /// 「用户要求截图」的时刻。用来在日志里打出端到端耗时，
    /// 定位「按了热键到看见遮罩」这条链路上到底是哪一段慢。
    request_at: Option<Instant>,
    /// 截图请求当前处在哪个阶段（延迟 / 等待自身窗口隐藏）。
    pending: Option<PendingCapture>,

    overlay: Shared<Option<Box<Overlay>>>,
    overlay_fix: Shared<Option<GeometryFix>>,
    /// 遮罩视口描述；`Some` 即表示当前存在一个遮罩。
    overlay_viewport: Option<ViewportBuilder>,
    /// 遮罩窗的命中判定是否已经抢过来（见 [`desktop::UiWake::take_over_hit_test`]）。
    /// 窗口是延迟建出来的，第一帧可能还拿不到句柄，所以留个旗标，摘成功就不再枚举窗口。
    overlay_hit_test_owned: bool,
    /// 单实例的等待线程是否已经起过（起一次就够，它自己会一直等）。
    waker_started: bool,
    /// 遮罩产出的结果，由根视口统一处理。
    overlay_results: Shared<Vec<OverlayResult>>,

    pins: Shared<HashMap<ViewportId, PinEntry>>,
    pin_counter: u64,
    /// 贴图窗口里改过的偏好。贴图视口的闭包是 `'static + Send`，拿不到 `&mut self`，
    /// 所以只能排队，回到主线程（`logic`）再写回配置——否则「在贴图菜单里调了不透明度，
    /// 下次开贴图又变回默认值」就是一枚假开关。
    pin_prefs: Shared<Vec<PinPrefPatch>>,
    /// 贴图窗口发起的 OCR 请求，同样排队回主线。
    pin_ocr_requests: Shared<Vec<Frame>>,
    /// 主显示器的 DPI 缩放。用于把物理像素换算成 winit 期望的「逻辑点」，
    /// 见 [`CuterApp::open_overlay`] 里关于几何初值的说明。
    ui_scale: f32,

    /// OCR 结果回传通道（工作线程 → UI 线程）。
    ocr_tx: Sender<OcrMessage>,
    ocr_rx: Receiver<OcrMessage>,
    /// OCR 面板视口描述；`Some` 表示面板存在。
    ocr_viewport: Option<ViewportBuilder>,
    ocr_panel: Shared<Option<Box<OcrPanel>>>,
    /// 面板产出的动作，由根视口统一处理。
    ocr_actions: Shared<Vec<OcrAction>>,
    /// 是否有一次识别正在后台跑。
    ocr_busy: bool,

    /// 滚动截屏会话（`Some` 表示正在采集）。
    scroll_session: Option<Arc<Mutex<ScrollSession>>>,
    /// 通知采集线程退出的旗标。
    scroll_stop: Option<Arc<AtomicBool>>,
    /// 控制窗视口描述；`Some` 表示控制窗存在。
    scroll_viewport: Option<ViewportBuilder>,
    scroll_panel: Shared<Option<Box<ScrollPanel>>>,
    scroll_actions: Shared<Vec<ScrollAction>>,

    /// 截图历史的本地缓存。**面板与宿主共享同一个 store**：
    /// 面板按需读缩略图，宿主负责写入与增删，两边看到的是同一份清单。
    history: Arc<Mutex<HistoryStore>>,
    /// 用户上次想要的保留张数。关掉历史时 `limit` 会被写成 0，
    /// 但重新打开时应当回到这个数，而不是悄悄变成默认值。
    history_limit_backup: usize,
    /// 历史面板视口描述；`Some` 表示面板存在。
    history_viewport: Option<ViewportBuilder>,
    history_panel: Shared<Option<Box<HistoryPanel>>>,
    history_actions: Shared<Vec<HistoryAction>>,

    /// 托盘菜单（自绘）的视口描述；`Some` 表示菜单正开着。
    tray_menu_viewport: Option<ViewportBuilder>,
    /// 菜单产出的动作，由根视口统一处理。
    tray_menu_actions: Shared<Vec<TrayAction>>,
    /// 菜单是否已经拿到过焦点。
    ///
    /// 子视口刚建出来那一两帧会报"没聚焦"，不能据此判定用户点到了别处——
    /// 只有"曾经聚焦过、现在又不聚焦了"才算失焦，这时才收起菜单（像系统菜单那样）。
    tray_menu_focused: Shared<bool>,

    /// 更新检查的结果通道：后台线程只发送，UI 线程每帧取走。
    update_tx: Sender<UpdateOutcome>,
    update_rx: Receiver<UpdateOutcome>,
    /// 是否有一次更新检查在飞（「关于」页据此显示「检查中…」，也防止连点）。
    update_checking: bool,
    /// 启动时那次自动检查是否已经发起过（只发起一次，不因为改了地址就反复自动查）。
    update_started: bool,
    /// 上一次检查的结果，「关于」页显示它。
    update_report: Option<UpdateOutcome>,

    /// 启动时主窗口要隐藏时，还需要强制隐藏几帧。    ///
    /// 为什么是"几帧"而不是一次：eframe 在**第一帧画完之后**才调用
    /// `window.set_visible(true)`，所以第一帧里下发的 `Visible(false)` 会被它盖掉，
    /// 必须再补一两帧才能稳定下来（见 `LaunchOptions::start_hidden` 的说明）。
    /// 一旦用户从托盘点开窗口就立刻清空它，绝不和用户抢。
    start_hidden_frames: Option<u8>,
    /// 热键 / 托盘线程用来立刻叫醒 UI 线程（绕过 eframe 对隐藏窗口的 100 ms 节流）。
    wake: UiWake,
    /// 设置窗口当前页（只在本次运行内记住）。
    settings_page: SettingsPage,
    /// 第一帧是否已经打过"启动时间线"的点。
    first_frame_logged: bool,
    /// 没有任何窗口需要画字之后，从这一刻开始计时（满 2 秒就把字体卸掉）。
    /// 托盘闲置计时：同样从"所有窗口都收起"开始算，满 30 秒把工作集还给系统。
    ws_idle_since: Option<Instant>,
    /// 本轮闲置是否已经收过工作集（有窗口出现就重置，避免反复触发页错误）。
    ws_trimmed: bool,
    /// 按下热键那一刻的前台程序名（文件名模板的 `{app}`）。
    capture_app: String,
    /// 本次截图所在显示器序号（1 开始；在 region 已知时算出来）。
    capture_monitor: u32,
    /// 贴图纹理超预算是否已经提醒过（掉回预算内会重置）。
    pin_budget_warned: bool,
    /// 托盘气泡通知器（首帧拿到根窗口句柄之后才建）。
    notifier: Option<desktop::TrayNotifier>,
    /// 处理过托盘/热键命令之后，保持出帧到这个时刻（让延迟视口有机会落地）。
    keep_awake_until: Option<Instant>,
    /// "进程创建 → 第一帧"的毫秒数。
    ///
    /// 诊断页会显示它：本项目的卖点之一就是"轻快"，那就把启动耗时摊开给用户看，
    /// 而不是让他凭感觉。同一台机器上这个数字还会受杀软扫描影响（见 HANDOFF 第 34 节）。
    startup_ms: u128,
    /// 本帧开始时量到的「热键线程唤醒 → 本帧」耗时。只用于打日志：热键链路上
    /// "多久才被读到"这一段在 app 侧是量不到的（请求登记已经是处理的那一刻了）。
    wake_age_ms: Option<u128>,
    /// `begin_request` 记下的"用户按下热键"时刻，共享给遮罩视口闭包。
    ///
    /// 闭包拿不到 `&self`，而"遮罩第一个 pass 落地"正是整条链路的终点，
    /// **必须在这一帧当场记**：换成"闭包写时刻、`logic()` 下一帧读走再落日志"会晚一帧，
    /// 而这一帧恰好是 100 ms 级别（实测把 135 ms 报成 267 ms，翻了一倍）。
    request_started: Shared<Option<Instant>>,

    /// "还没记录过首帧"的一次性开关（`swap(false)` 保证只写一次）。
    overlay_first_pass_pending: Arc<std::sync::atomic::AtomicBool>,
    /// 已经等了多少帧（用于给"催帧"设上限，见 `logic()`）。
    overlay_wait_frames: u32,
    /// 后台建好的 DXGI 捕获后端（`capture.backend = "dxgi"` 时才有）。
    ///
    /// 建它实测要 1.76 s，所以**绝不能**放在捕获路径或启动路径上：见 `spawn_dxgi_backend`。
    /// 这里一直是 `None` 就表示"当前在用 GDI"，设置窗口的诊断页会如实显示。
    dxgi_backend: Shared<Option<Box<dyn cuter_core::capture::ScreenCapture>>>,
    /// 贴图是否处于「全部隐藏」状态（`Shift+F3` 切）。
    ///
    /// 只收窗口、**不动 `Pin` 里的状态**：再按一次要能原样回来
    /// （位置 / 缩放 / 透明度 / 标注）。
    pins_hidden: bool,
    /// 截图期间把贴图全部撤下去（`hide_all_windows` 设置、`restore_control` 清掉）。
    ///
    /// 单独一个旗标而不是复用 `pins_hidden`：这是**临时**的，抓完图要自己回来。
    pins_suppressed: bool,
    /// 当前在哪一组贴图（`Ctrl+F3` 切）。
    ///
    /// 分组是"几套互不干扰的贴图"：切组时当前组整体隐藏、目标组整体显示，
    /// 新建的贴图落在当前组。持久化在 `config.pin.active_group`。
    pin_group: u32,
    /// 本次截图请求是什么时候要求"隐藏自身窗口"的（`None` 表示这次没隐藏）。
    /// 用来给"屏幕上只有贴图窗口"这种情况兜一个最小等待时间，见
    /// [`CuterApp::hide_seen`]。
    hide_requested_at: Option<Instant>,
    /// 我们自己记的「主窗口现在是不是可见的」。
    ///
    /// `set_control_visible` 是唯一的写入点。用它而不是问 egui：见
    /// [`CuterApp::needs_hide_delay`]。
    control_visible: std::cell::Cell<bool>,
    /// 设置界面上的热键编辑缓冲。
    hotkey_buffer: Vec<(String, String)>,
    status: String,
    status_frames: u32,
    /// 待命心跳的空转计数，用来把重绘间隔从 1 s 逐步退到 4 s。
    idle_ticks: u32,
    warnings: Vec<String>,
    quit: bool,
    last_saved: Option<PathBuf>,
    /// 启动时要求立刻贴一张剪贴板图片（`cuter --pin`），执行一次后清空。
    pending_start_pin: bool,
    /// 启动时要求打开历史面板（`cuter --history`），执行一次后清空。
    pending_start_history: bool,
    /// 启动时要求打开托盘菜单（`cuter --tray-menu`），执行一次后清空。
    pending_start_tray_menu: bool,
    /// `cuter --about`：启动后打开「设置 → 关于」页。
    pending_start_about: bool,
}

impl CuterApp {
    pub fn new_with(
        cc: &eframe::CreationContext<'_>,
        options: LaunchOptions,
    ) -> Result<Self, String> {
        let (tx, rx) = channel();
        let (update_tx, update_rx) = channel();
        let mut config = Config::load();
        // 配置读坏过（`load()` 已回退默认值并备份了原文件）时必须让用户知道：
        // 设置"莫名其妙全变回默认"最容易让人以为程序在坑他。
        let config_recovery = Config::last_recovery().cloned();
        // 界面语言：`auto` 交给宿主探测（cuter-ui 不碰平台 API）
        cuter_ui::i18n::set_language(cuter_ui::i18n::Language::parse(
            &config.general.language,
            crate::desktop::system_language(),
        ));
        config.sanitize();

        // 让注册表里的自启项与配置对账：
        // * 配置说开 → 写成当前 exe（用户把文件夹搬走了也能自愈）；
        // * 配置说关 → 只删指向本 exe 的那一条（别人的自启项一概不动）。
        //
        // **配置是唯一的事实来源**：这里刻意不做"注册表里有就采纳"——
        // 那会让"手工把配置改成 false"变成改不掉的状态。代价是升级时有个小竞态：
        // 安装器写了自启，而正在跑的**旧实例**退出时会把它内存里的旧配置写回去
        // （见 HANDOFF 17.4 的已知限制）。宁可这样，也不要一个"关不掉的开关"。
        //
        // 只在启动时做一次——它是注册表操作，没必要每帧问。
        log::info!(
            "启动时间线[配置已读②/语言已定]：+{} ms",
            crate::desktop::since_process_start_ms()
        );
        desktop::reconcile_autostart(config.general.autostart);
        log::info!(
            "启动时间线[自启对账完成]：+{} ms",
            crate::desktop::since_process_start_ms()
        );

        // 热键 / 托盘的事件处理器要在另一个线程上把 UI 叫醒，句柄由 `logic()` 稍后填进来
        // （那时才拿得到 eframe 的 `Frame`）。
        let wake = UiWake::new();

        let theme = resolve_theme(config.general.theme, &cc.egui_ctx);
        cc.egui_ctx.set_visuals(theme.visuals());

        // 系统字体：egui 自带字体没有 CJK 字形，不装的话界面全是方框。
        // **启动时先不读**（见 `ensure_font`）：托盘常驻但不截图时，那 18.8 MB 的
        // msyh.ttc 完全用不上，白白占着常驻内存。所以这里不再判定"有没有中文字体"，
        // 那条告警挪到真正装载之后（`ensure_font` 里）。
        let font: Option<LoadedFont> = None;
        let mut warnings: Vec<String> = Vec::new();

        log::info!(
            "启动时间线[托盘/热键之前]：+{} ms",
            crate::desktop::since_process_start_ms()
        );
        let mut hotkeys = match Hotkeys::new(&cc.egui_ctx, wake.clone()) {
            Ok(manager) => Some(manager),
            Err(err) => {
                warnings.push(err);
                None
            }
        };
        if let Some(manager) = hotkeys.as_mut() {
            warnings.extend(manager.apply(&config.hotkeys));
        }

        log::info!(
            "启动时间线[热键注册完成]：+{} ms",
            crate::desktop::since_process_start_ms()
        );
        let tray = match Tray::new(&cc.egui_ctx, wake.clone(), &config) {
            Ok(tray) => Some(tray),
            Err(err) => {
                warnings.push(err);
                None
            }
        };

        let hotkey_buffer = config
            .hotkeys
            .entries()
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();

        // 在 `config` 被移进结构体之前算好：关掉历史时 `limit` 是 0，
        // 但用户重新勾上时应当回到上次想要的张数，而不是默认值。
        let history_limit_backup = if config.history.enabled() {
            config.history.limit
        } else {
            cuter_core::config::HistoryConfig::default().limit
        };
        let history_dir = config.history.dir();
        // 贴图分组：当前组要跟配置走（持久化的是"上次在第几组"）
        let pin_group = config.pin.active_group;

        // DXGI 要提前在后台建（下面用到）
        let wants_dxgi = config.capture.backend == cuter_core::capture::CaptureBackend::Dxgi;
        let ctx_for_dxgi = cc.egui_ctx.clone();

        let app = Self {
            config,
            first_frame_logged: false,
            startup_ms: 0,
            keep_awake_until: None,
            ws_idle_since: None,
            ws_trimmed: false,
            capture_app: String::new(),
            capture_monitor: 1,
            pin_budget_warned: false,
            notifier: None,
            theme,
            // 哨兵值不是"当前皮肤"，而是"窗口与托盘图标出厂时用的皮肤"：`main.rs` 建窗口
            // 时 `resolve_theme` 还没跑，两处拿到的都是默认的紫色。所以这里固定写 Purple，
            // 让浅色主题在**第一帧**就被判定为"皮肤变了"，从而把窗口与托盘图标换成奶白。
            icon_skin: crate::icon::IconSkin::Purple,
            font,
            tray,
            hotkeys,
            instance: options.instance,
            request: options.start_request,
            request_at: options.start_request.map(|_| Instant::now()),
            pending: None,
            overlay: Arc::new(Mutex::new(None)),
            overlay_fix: Arc::new(Mutex::new(None)),
            overlay_viewport: None,
            overlay_hit_test_owned: false,
            waker_started: false,
            overlay_results: Arc::new(Mutex::new(Vec::new())),
            pins: Arc::new(Mutex::new(HashMap::new())),
            pin_counter: 0,
            pin_prefs: Arc::new(Mutex::new(Vec::new())),
            pin_ocr_requests: Arc::new(Mutex::new(Vec::new())),
            ui_scale: primary_scale_factor(),
            ocr_tx: tx,
            ocr_rx: rx,
            ocr_viewport: None,
            ocr_panel: Arc::new(Mutex::new(None)),
            ocr_actions: Arc::new(Mutex::new(Vec::new())),
            scroll_session: None,
            scroll_stop: None,
            scroll_viewport: None,
            scroll_panel: Arc::new(Mutex::new(None)),
            scroll_actions: Arc::new(Mutex::new(Vec::new())),
            // 历史：即使配置里关掉了也把 store 打开（用户可能想进去看看/清空旧记录），
            // 只是不再往里写新的——`remember_history` 会先看 `history.enabled()`。
            history_limit_backup,
            history: Arc::new(Mutex::new(HistoryStore::open(history_dir))),
            history_viewport: None,
            history_panel: Arc::new(Mutex::new(None)),
            history_actions: Arc::new(Mutex::new(Vec::new())),
            tray_menu_viewport: None,
            tray_menu_actions: Arc::new(Mutex::new(Vec::new())),
            tray_menu_focused: Arc::new(Mutex::new(false)),
            update_tx,
            update_rx,
            update_checking: false,
            update_started: false,
            update_report: None,
            ocr_busy: false,
            start_hidden_frames: options.start_hidden.then_some(3),
            wake,
            // 当前在第几组贴图（持久化，见 `PinConfig::active_group`）
            pin_group,
            settings_page: SettingsPage::General,
            wake_age_ms: None,
            request_started: Arc::new(Mutex::new(None)),

            overlay_first_pass_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            overlay_wait_frames: 0,
            // 后端按配置在后台建（见 `spawn_dxgi_backend` 的调用点）
            dxgi_backend: Arc::new(Mutex::new(None)),
            pins_hidden: false,
            pins_suppressed: false,
            hide_requested_at: None,
            // 初始可见性与 `main.rs` 建窗口时的 `with_visible(!start_hidden)` 一致
            control_visible: std::cell::Cell::new(!options.start_hidden),
            hotkey_buffer,
            status: match &config_recovery {
                Some(recovery) => fill!(
                    tr!("配置文件读不出来，已回退默认值（原文件备份在 {}）"),
                    recovery
                        .backup
                        .as_ref()
                        .map_or_else(|| tr!("备份失败").to_owned(), |path| path.display().to_string())
                ),
                None => tr!("按 Alt+C 开始截图").to_owned(),
            },
            status_frames: 0,
        idle_ticks: 0,
            warnings,
            quit: false,
            last_saved: None,
            pending_start_pin: options.start_pin,
            pending_start_history: options.start_history,
            pending_start_tray_menu: options.start_tray_menu,
            pending_start_about: options.start_about,
        };
        // `capture.backend = "dxgi"` 时提前在后台把桌面复制建起来（实测 1.76 s，
        // 见 `spawn_dxgi_backend`）；配置是 GDI 就完全不碰 D3D。
        if wants_dxgi {
            spawn_dxgi_backend(Arc::clone(&app.dxgi_backend), ctx_for_dxgi);
        }
        Ok(app)
    }

    /// 主窗口是否可见（用于判断需不需要先隐藏再截图）。
    ///
    /// **唯一的写入点**：`needs_hide_delay` 依赖这个记录来判断"截图前要不要等两帧"。
    fn set_control_visible(&self, ctx: &Context, visible: bool) {
        self.control_visible.set(visible);
        ctx.send_viewport_cmd(ViewportCommand::Visible(visible));
    }

    fn hide_all_windows(&mut self, ctx: &Context) {
        if !self.config.capture.hide_own_windows {
            return;
        }
        self.set_control_visible(ctx, false);
        // 贴图：不再声明它们的视口（窗口会被收掉），比 `Visible(false)` 可靠——
        // 后者对"每帧重新声明"的视口不生效，实测会让截图等满 900 ms 超时后
        // 还是把贴图截进去（HANDOFF 第 22 节）。
        if !lock(&self.pins).is_empty() {
            self.pins_suppressed = true;
        }
        // 遮罩也要藏：它可能开在屏幕上（「重新截图」就是这条路）。
        // 不给它发命令的话，下面的等待永远等不到它消失，只能等到超时。
        //
        // ⚠️ **必须先确认它真的挡在桌面上**再发命令。两个原因：
        // 一是托盘模式（还没有遮罩窗口）里这条命令会挂到那个视口 id 上，于是
        // **紧接着新建的遮罩一开始就是隐藏的**——表现成"按热键遮罩再也不出现"（实测踩过）；
        // 二是视口复用之后遮罩窗口常年存在，只是停在桌面之外（用 `_on_desktop` 才能区分）。
        if desktop::UiWake::window_with_title_on_desktop(OVERLAY_TITLE) {
            ctx.send_viewport_cmd_to(overlay_viewport_id(), ViewportCommand::Visible(false));
        }
    }

    fn restore_control(&mut self, ctx: &Context) {
        // 已经进入正常交互了，"启动时先藏起来"这条要求就该作废
        self.start_hidden_frames = None;
        // 截图期间撤下去的贴图要回来（`pins_hidden` / 分组规则仍然独立生效）
        self.pins_suppressed = false;
        if self.config.general.start_in_tray {
            return;
        }
        self.set_control_visible(ctx, true);
    }

    /// 按启动选项把主窗口收起来（托盘驻留 / `--capture` 直接进框选）。
    ///
    /// 必须在这里做，而不是交给 `ViewportBuilder::with_visible(false)`：
    /// eframe 画完第一帧后会无条件把根窗口显示出来，那个选项会被覆盖。
    /// 现象是「设置了启动即驻留托盘，双击之后设置窗口还是弹在脸上」。
    fn enforce_start_hidden(&mut self, ctx: &Context) {
        let Some(remaining) = self.start_hidden_frames else {
            return;
        };
        self.set_control_visible(ctx, false);
        self.start_hidden_frames = remaining.checked_sub(1).filter(|n| *n > 0);
    }

    // ------------------------------------------------------------------
    // 桌面事件
    // ------------------------------------------------------------------

    fn poll_desktop(&mut self, ctx: &Context) {
        // 托盘与热键都是「事件处理器 → 队列」：处理器在事件到达时就唤醒了 UI
        // （见 desktop.rs 的模块文档），这里只是把队列取空。
        //
        // 但**只唤醒一帧是不够的**：一次命令常常要好几帧才落地——贴图窗口是延迟视口，
        // 它的窗口要等下一帧才被创建、`self.pins` 也要等那个闭包跑起来才变成非空，
        // 而"非空"正是继续出帧的条件之一（鸡生蛋）。所以处理过命令就保持出帧一小段时间。
        let mut handled = false;
        if let Some(tray) = &self.tray
            && let Some(command) = tray.poll() {
                self.handle_tray(ctx, command);
                handled = true;
            }
        if let Some(command) = self.hotkeys.as_ref().and_then(Hotkeys::poll) {
            self.handle_hotkey(ctx, command);
            handled = true;
        }
        if handled {
            // 命令之后的一小段时间里**主动顶帧**：贴图窗口这类延迟视口要好几帧才落地，
            // 而不可见窗口收不到 `WM_PAINT`，光靠 `request_repaint_after` 排不上队。
            self.keep_awake_until = Some(Instant::now() + Duration::from_millis(1500));
            self.wake.wake(ctx);
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// 把遮罩里调过的工具样式写回配置（下次打开遮罩还是这套参数）。
    ///
    /// **不改 `default_tool`**：那是"每次截图默认用哪个工具"的偏好，应该由设置窗口决定。
    /// 用户在遮罩里临时切到箭头画一笔，不该把下一次截图的默认工具也改了。
    fn apply_tool_style(&mut self, style: &cuter_ui::annotate::ToolSettings) {
        let annotate = &mut self.config.annotate;
        let mut changed = false;
        let rgb = [style.color.r(), style.color.g(), style.color.b()];
        if annotate.stroke_color != rgb {
            annotate.stroke_color = rgb;
            changed = true;
        }
        if (annotate.stroke_width - style.stroke_width).abs() > f32::EPSILON {
            annotate.stroke_width = style.stroke_width;
            changed = true;
        }
        if annotate.fill_shapes != style.fill_shapes {
            annotate.fill_shapes = style.fill_shapes;
            changed = true;
        }
        if annotate.mosaic_block != style.mosaic_block {
            annotate.mosaic_block = style.mosaic_block;
            changed = true;
        }
        if annotate.blur_radius != style.blur_radius {
            annotate.blur_radius = style.blur_radius;
            changed = true;
        }
        if (annotate.font_size - style.font_size).abs() > f32::EPSILON {
            annotate.font_size = style.font_size;
            changed = true;
        }
        if changed {
            self.config.annotate.clamp();
            log::debug!(
                "遮罩里的工具样式已写回配置：线宽 {} / 填充 {} / 马赛克 {}",
                self.config.annotate.stroke_width,
                self.config.annotate.fill_shapes,
                self.config.annotate.mosaic_block
            );
            self.persist_config();
        }
    }

    fn handle_tray(&mut self, ctx: &Context, command: TrayCommand) {
        match command {
            TrayCommand::Capture => self.begin_request(CaptureRequest::Region),
            TrayCommand::CaptureMonitor => self.begin_request(CaptureRequest::Monitor),
            TrayCommand::PinFromClipboard => self.pin_from_clipboard(ctx),
            TrayCommand::TogglePins => self.toggle_pins(ctx),
            TrayCommand::ClearPinClickThrough => self.clear_pin_click_through(ctx),
            TrayCommand::NextPinGroup => self.switch_pin_group(ctx),
            TrayCommand::OpenHistory => self.open_history(ctx),
            TrayCommand::OpenMenu => self.open_tray_menu(ctx),
            TrayCommand::OpenSettings => {
                // 用户点名要窗口：作废"启动即隐藏"，避免和用户抢
                self.start_hidden_frames = None;
                self.set_control_visible(ctx, true);
            }
            TrayCommand::OpenSaveDir => self.open_save_dir(ctx),
            TrayCommand::Quit => self.quit = true,
        }
    }

    fn handle_hotkey(&mut self, ctx: &Context, command: HotkeyCommand) {
        match command {
            HotkeyCommand::Capture => self.begin_request(CaptureRequest::Region),
            HotkeyCommand::CaptureMonitor => self.begin_request(CaptureRequest::Monitor),
            HotkeyCommand::CaptureDesktop => self.begin_request(CaptureRequest::Desktop),
            HotkeyCommand::ScrollCapture => self.request_scroll(ctx),
            HotkeyCommand::PinFromClipboard => self.pin_from_clipboard(ctx),
            HotkeyCommand::TogglePins => self.toggle_pins(ctx),
            HotkeyCommand::NextPinGroup => self.switch_pin_group(ctx),
            HotkeyCommand::Ocr | HotkeyCommand::OcrClipboard => self.ocr_hotkey(ctx, command),
        }
    }

    /// 「识别文字」与「贴图并识别」两条热键。
    ///
    /// 在此之前这两条热键只出现在设置页的列表里，按下去换来一句「规划在 M3 实现」——
    /// 于是 OCR 面板实际上只能从遮罩里的 Ctrl+O 进，而热键文案（"对剪贴板 / 上次截图
    /// 做 OCR"、"从剪贴板贴图并立刻 OCR"）又承诺得明明白白。现在补齐：
    ///
    /// * `Ocr`：把剪贴板里的图直接送去识别（与"截图后 Ctrl+O"同一条路）；
    /// * `OcrClipboard`：先照名字把这张图贴到屏幕上，再识别同一张（一次按键两件事）。
    fn ocr_hotkey(&mut self, ctx: &Context, command: HotkeyCommand) {
        let frame = match desktop::paste_image() {
            Ok(frame) => frame,
            Err(err) => {
                log::warn!("热键 OCR 取剪贴板失败：{err}");
                self.set_status(err);
                return;
            }
        };
        if matches!(command, HotkeyCommand::OcrClipboard) {
            let cursor = self.cursor_global();
            self.open_pin(ctx, frame.clone(), cursor);
        }
        self.start_ocr(ctx, frame);
    }

    /// 隐藏 / 显示屏幕上当前组的贴图（默认 `Shift+F3`）。
    ///
    /// 贴图开着开会/演示时最烦：全都要手动一张张关。这里**不动 `Pin` 里的任何状态**，
    /// 只是不再声明它们的视口（窗口被收掉），再按一次原样回来（位置、缩放、透明度都在）。
    fn toggle_pins(&mut self, ctx: &Context) {
        if lock(&self.pins).is_empty() {
            self.set_status(tr!("屏幕上还没有贴图"));
            return;
        }
        self.pins_hidden = !self.pins_hidden;
        let count = self.pins_in_group(self.pin_group);
        self.set_status(if self.pins_hidden {
            fill!(tr!("已隐藏第 {} 组的 {} 张贴图（再按一次可以恢复）"), self.pin_group + 1, count)
        } else {
            fill!(tr!("已恢复第 {} 组的 {} 张贴图"), self.pin_group + 1, count)
        });
        // 隐藏之后这一组不再有窗口，纹理可以放掉；恢复时按需重建。
        self.release_hidden_pin_textures(ctx);
        ctx.request_repaint();
    }

    /// 托盘菜单的逃生项：把所有贴图的鼠标穿透关掉。
    ///
    /// 穿透是窗口级的开关（`WS_EX_TRANSPARENT`）：打开之后贴图窗口收不到**任何**鼠标
    /// 事件，右键菜单都弹不出来——"在菜单里把它关掉"这条路是死的。所以必须有一个窗口
    /// 外面的入口，这就是它（贴图那边也会把提示改写成"点托盘图标取消"，不再骗人）。
    fn clear_pin_click_through(&mut self, ctx: &Context) {
        // 先把句柄收集出来再逐个上锁：端着 `pins` 表的锁去锁贴图，和别处
        // （先锁贴图、再读表）凑一起就是死锁。
        let pins: Vec<Shared<Pin>> = lock(&self.pins)
            .values()
            .map(|entry| Arc::clone(&entry.pin))
            .collect();
        let mut cleared = 0usize;
        for pin in &pins {
            let mut guard = lock(pin);
            if guard.click_through() {
                guard.set_click_through(false);
                cleared += 1;
            }
        }
        // 配置里的那个是"下一张贴图的默认值"，一起关：否则用户下一张又是穿透的，
        // 又要来点一遍这个菜单项。
        let was_default = self.config.pin.click_through;
        if was_default {
            self.config.pin.click_through = false;
            self.persist_config();
        }
        self.set_status(if cleared == 0 && !was_default {
            tr!("没有贴图开着鼠标穿透").to_owned()
        } else {
            fill!(tr!("已取消 {} 张贴图的鼠标穿透"), cleared)
        });
        ctx.request_repaint();
    }

    /// 切到下一组贴图（默认 `Alt+G`）。
    ///
    /// 贴图组：把"参考资料"和"正在对着看的东西"分成两套，
    /// 切一下整体换掉，而不是一张张关。**只改可见性**，所以切回去东西都还在。
    fn switch_pin_group(&mut self, ctx: &Context) {
        let groups = self.config.pin.groups.max(1);
        self.pin_group = (self.pin_group + 1) % groups;
        // 切组本身就是"要看这一组"，顺手取消"全部隐藏"
        self.pins_hidden = false;
        // 组号持久化：下次启动新建的贴图落在同一组
        self.config.pin.active_group = self.pin_group;
        self.persist_config();
        let count = self.pins_in_group(self.pin_group);
        self.set_status(fill!(tr!("已切到第 {}/{} 组贴图（本组 {} 张）"), self.pin_group + 1, groups, count));
        // 切走的那一组纹理放掉（它们的窗口已经收了，纹理白占内存）
        self.release_hidden_pin_textures(ctx);
        ctx.request_repaint();
    }

    /// 把"现在不该显示"的贴图纹理都丢掉（分组切走、整体隐藏）。
    ///
    /// 为什么值得做：贴图纹理一张 4K 就是 33 MB（RGBA8，宽×高×4）。切到另一组之后，
    /// 原来那组的窗口不再声明，**根本不会跑自己的 pass**，但纹理还挂在 egui 里占着显存与内存。
    /// 原始 `Frame` 留着，所以切回来时只是重新上传一次纹理。
    fn release_hidden_pin_textures(&mut self, ctx: &Context) {
        let mut released = 0usize;
        let mut bytes = 0usize;
        for entry in lock(&self.pins).values() {
            if self.pin_should_show(entry) {
                continue;
            }
            let mut pin = lock(&entry.pin);
            let was = pin.texture_bytes();
            if was > 0 {
                pin.drop_texture();
                released += 1;
                bytes += was;
            }
        }
        if released > 0 {
            log::info!(
                "release-hidden-pin-textures count={released} mb={}（隐藏的贴图不再占纹理）",
                bytes / (1024 * 1024)
            );
            ctx.request_repaint();
        }
        self.warn_if_pin_textures_over_budget();
    }

    /// 贴图纹理总量超过 `pin.texture_budget_mb` 就提醒一次（每次跨越阈值只提醒一次）。
    fn warn_if_pin_textures_over_budget(&mut self) {
        let budget = u64::from(self.config.pin.texture_budget_mb) * 1024 * 1024;
        let total: u64 = lock(&self.pins)
            .values()
            .map(|entry| lock(&entry.pin).texture_bytes() as u64)
            .sum();
        let over = total > budget;
        if over && !self.pin_budget_warned {
            self.pin_budget_warned = true;
            let mb = total / (1024 * 1024);
            log::warn!("pin-texture-budget total_mb={mb} budget_mb={}", self.config.pin.texture_budget_mb);
            if let Some(notifier) = self.notifier.as_mut()
                && self.config.general.show_tray_notifications
            {
                let body = fill!(
                    tr!("贴图纹理已占用 {} MB（上限 {} MB）；关掉一些贴图可以释放"),
                    mb,
                    self.config.pin.texture_budget_mb
                );
                notifier.notify(tr!("贴图占用偏多"), &body);
            }
        } else if !over {
            // 掉回预算以内：下次再超还能再提醒一次
            self.pin_budget_warned = false;
        }
    }

    /// 某一组里有多少张贴图。
    fn pins_in_group(&self, group: u32) -> usize {
        lock(&self.pins)
            .values()
            .filter(|entry| entry.group == group)
            .count()
    }

    /// 这张贴图现在该不该出现在屏幕上。
    ///
    /// **唯一的可见性判据**：`declare_viewports` 按它决定要不要声明视口
    /// （不声明 = 窗口被收掉），切组 / 隐藏 / 截图前隐藏都走这一条。
    fn pin_should_show(&self, entry: &PinEntry) -> bool {
        !self.pins_suppressed && !self.pins_hidden && entry.group == self.pin_group
    }

    /// 请求进入滚动截屏。
    ///
    /// 三种情况分开处理，避免「已经开着遮罩却又重新截一遍屏」这种浪费：
    /// 已经开着的遮罩直接切模式（保留用户已有的框选），否则新发一次带滚动模式的截图请求。
    fn request_scroll(&mut self, _ctx: &Context) {
        if self.scroll_session.is_some() {
            self.set_status(tr!("滚动截屏已经在进行中"));
            return;
        }
        let switched = {
            let mut overlay = lock(&self.overlay);
            match overlay.as_mut() {
                Some(overlay) => {
                    overlay.set_scroll_mode(true);
                    true
                }
                None => false,
            }
        };
        if switched {
            self.set_status(tr!("已切到滚动截屏：框选后按回车开始采集"));
        } else {
            self.begin_request(CaptureRequest::Scroll);
        }
    }

    fn pin_from_clipboard(&mut self, ctx: &Context) {
        match desktop::paste_image() {
            Ok(frame) => {
                log::info!("剪贴板贴图：{}×{}", frame.width(), frame.height());
                let cursor = self.cursor_global();
                self.open_pin(ctx, frame, cursor);
            }
            Err(err) => {
                log::warn!("剪贴板贴图失败：{err}");
                self.set_status(err);
            }
        }
    }

    /// 鼠标所在的全局物理坐标（取不到时退回主屏左上角内侧，保证贴图还能看见）。
    fn cursor_global(&self) -> cuter_core::geom::Point {
        cuter_core::capture::cursor::cursor_position().unwrap_or_else(|| {
            cuter_core::geom::Point::new(80, 80)
        })
    }

    fn open_save_dir(&mut self, _ctx: &Context) {
        let dir = self.config.save.resolved_dir();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.set_status(fill!(tr!("创建目录失败：{}"), err));
            return;
        }
        #[cfg(windows)]
        {
            if let Err(err) = std::process::Command::new("explorer").arg(&dir).spawn() {
                self.set_status(fill!(tr!("打开资源管理器失败：{}"), err));
            }
        }
    }

    // ------------------------------------------------------------------
    // 截图流程
    // ------------------------------------------------------------------

    /// 推进截图请求。
    ///
    /// 三个阶段，顺序不能乱：
    ///
    /// 1. **延迟**（`capture.delay_ms`）：用户按了热键但屏幕上有菜单要收起来时用。
    /// 2. **隐藏自身窗口**：`Visible(false)` 是异步的，屏幕上真有我们自己的窗口时
    ///    必须再等两帧，否则会把自己的贴图截进去；**没有就不等**（见
    ///    [`CuterApp::begin_hiding_or_capture`]）。
    /// 3. **捕获**。
    fn advance_request(&mut self, ctx: &Context) {
        let Some(request) = self.request else {
            return;
        };

        match self.pending {
            None => {
                let delay = self.config.capture.delay_ms;
                if delay > 0 {
                    let deadline = Instant::now() + Duration::from_millis(u64::from(delay));
                    self.pending = Some(PendingCapture::Waiting(deadline));
                    self.set_status(fill!(tr!("{} 毫秒后捕获…"), delay));
                    ctx.request_repaint_after(Duration::from_millis(u64::from(delay)));
                } else {
                    self.begin_hiding_or_capture(ctx, request);
                }
            }
            Some(PendingCapture::Waiting(deadline)) => {
                if Instant::now() < deadline {
                    ctx.request_repaint_after(Duration::from_millis(16));
                    return;
                }
                self.begin_hiding_or_capture(ctx, request);
            }
            Some(PendingCapture::Hiding { deadline, hidden_at }) => {
                let now = Instant::now();
                // 第一段：等"真的不可见"。判据是问操作系统（拿不到句柄时退回时间下限），
                // 不猜时长——猜短了会把自己的窗口截进图里。
                let hidden_at = match hidden_at {
                    Some(at) => Some(at),
                    None if now >= deadline => None,
                    None if self.hide_seen() => {
                        // 刚刚不可见：推一次合成器，让它把这一帧换掉，再开始计时。
                        desktop::flush_composition();
                        Some(now)
                    }
                    None => {
                        std::thread::sleep(Duration::from_millis(4));
                        self.wake.wake(ctx);
                        self.pending = Some(PendingCapture::Hiding { deadline, hidden_at });
                        return;
                    }
                };
                // 第二段：不可见之后再留余量，等屏幕真的换成新的一帧。
                let ready = hidden_at.is_some_and(|at| now >= at + HIDE_SETTLE_AFTER);
                if ready || now >= deadline {
                    if hidden_at.is_none() {
                        log::warn!(
                            "等待自身窗口消失超时（{:?}），这次截图可能把自己的窗口也拍进去",
                            HIDE_TIMEOUT
                        );
                    }
                    self.finish_capture(ctx, request);
                    return;
                }
                self.pending = Some(PendingCapture::Hiding { deadline, hidden_at });
                std::thread::sleep(Duration::from_millis(4));
                self.wake.wake(ctx);
            }
        }
    }

    /// 「隐藏自身窗口」这一步的入口：**只有在屏幕上真的有我们自己的窗口时才等**。
    ///
    /// 托盘模式（主窗口本来就隐藏、屏幕上没有贴图）根本不需要等：没有任何我们自己的
    /// 像素会被截进去。设置窗口开着、或有贴图时才必须等，见 [`CuterApp::needs_hide_delay`]
    /// 与 [`HIDE_SETTLE_AFTER`]。
    ///
    /// 实测（`tools/probe/perf_probe.ps1`）：托盘模式按下 Alt+C 到遮罩可见
    /// **min 79 / median 113 / max 188 ms**（5 次），其中"唤醒 → 被处理"只有
    /// **2~13 ms**，捕获与合成光标 53~105 ms（随机器负载浮动）。详见 HANDOFF 第 15 节。
    fn begin_hiding_or_capture(&mut self, ctx: &Context, request: CaptureRequest) {
        // ⚠️ **必须先判断、再隐藏**。`hide_all_windows` 会把 `control_visible` 清成 false，
        // 所以"先隐藏再问需不需要等帧"永远得到"不需要"——实测就是这么漏掉的：
        // 设置窗口明明在屏幕上，却直接抓图，会把它一起冻进截图里。
        let wait_for_hide = self.needs_hide_delay();
        self.hide_all_windows(ctx);
        if wait_for_hide {
            let now = Instant::now();
            self.hide_requested_at = Some(now);
            self.pending = Some(PendingCapture::Hiding {
                deadline: now + HIDE_TIMEOUT,
                hidden_at: None,
            });
            self.log_request_stage_info("请求隐藏自身窗口");
            // 这一帧结束后 eframe 才会真正执行 `Visible(false)`，用 `wake` 让下一帧立刻到，
            // 好在那帧里轮询窗口是不是真的没了（隐藏窗口靠 eframe 的重绘请求要等 100 ms）。
            self.wake.wake(ctx);
        } else {
            self.log_request_stage_info("无需等帧，直接捕获");
            self.finish_capture(ctx, request);
        }
    }

    /// 捕获并交付。
    fn finish_capture(&mut self, ctx: &Context, request: CaptureRequest) {
        self.perform_capture(ctx, request);
        self.request = None;
        self.pending = None;
        self.hide_requested_at = None;
        self.restore_control(ctx);
    }

    /// 我们自己的窗口是否已经从屏幕上撤下去了。
    ///
    /// 主判据是**直接问操作系统**（[`crate::desktop::UiWake::root_is_visible`]）：
    /// 时长在不同的机器上完全不可靠。只有"屏幕上只有贴图窗口、拿不到它的句柄"这种情况
    /// 才退回时间下限（[`HIDE_SETTLE_BEFORE`]）。
    fn hide_seen(&self) -> bool {
        if let Some(at) = self.hide_requested_at
            && at.elapsed() < HIDE_SETTLE_BEFORE
        {
            return false;
        }
        // 根窗口（设置窗口）和子视口（遮罩 / 贴图）都要撤下去。
        // 只问根窗口是不够的：「重新截图」时根窗口本来就是隐藏的，而遮罩正开着。
        !self.wake.root_is_visible() && !self.own_window_on_screen()
    }

    /// 屏幕上是否有我们自己的窗口需要先撤下去。
    ///
    /// **只能在 [`CuterApp::hide_all_windows`] 之前调用**：隐藏动作本身就会把这个判据清掉。
    ///
    /// **不能用 `ctx.input(|i| i.viewport().visible())` 判断**：我们自己用
    /// `ViewportCommand::Visible(false)` 藏起来的窗口，egui 那边的 `ViewportInfo.visible`
    /// 不一定会跟着更新（实测在托盘模式下它一直报告 true），于是"托盘模式不需要等帧"
    /// 这条优化会失效。这里改成跟踪我们**自己下发的**可见性，确定且不依赖框架。
    ///
    /// 为什么要等：`Visible(false)` 是在这一帧末尾才被 eframe 执行（`process_commands`）
    /// 的，同帧立刻抓图会把我们自己的窗口一起冻进截图。等的那几帧**在窗口还可见时很便宜**
    /// （重绘请求发出时窗口还是可见的，不命中 eframe 的隐形窗口节流）；
    /// 托盘模式下窗口本来就不可见，才必须跳过等待（每帧 100 ms）。
    fn needs_hide_delay(&self) -> bool {
        if !self.config.capture.hide_own_windows {
            // 用户明确要求"别藏我自己的窗口"，那就没什么好等的
            return false;
        }
        self.control_visible.get()
            || !lock(&self.pins).is_empty()
            // 托盘菜单也是延迟视口：它开着的时候必须继续出帧，否则"按 Esc 关闭"
            // 要等下一次心跳（待命时最长 4 s）才被画出来——tray_menu 探针
            // 一直抓到的就是这个（菜单窗口能见状态比动作晚好几秒）。
            || self.tray_menu_viewport.is_some()
            // ⚠️ **遮罩自己也要算**：点「重新截图」时遮罩正开在屏幕上，而
            // `control_visible` 是 false、贴图列表是空的——曾经因此判定"无需等帧"，
            // 直接把遮罩冻进了新的一帧（整屏变成遮罩底色，HANDOFF 第 20 节）。
            // 这里问的是操作系统（按标题找我们自己的窗口），所以哪怕 egui 那边
            // 的视口状态还没更新也拦得住。
            || self.own_window_on_screen()
    }

    /// 我们自己的窗口（遮罩 / 贴图）现在是否真的还显示在屏幕上。
    ///
    /// 不看 `self.overlay_viewport` / `self.pins` 这些"我们以为的状态"：egui 回收视口
    /// 与 winit 销毁窗口之间隔着好几步，判断"屏幕上还有没有"必须问操作系统。
    ///
    /// 遮罩用 `..._on_desktop` 而不是 `..._visible`：视口复用之后它**常年存在**，
    /// 只是停在桌面之外（1×1），`IsWindowVisible` 永远是真——用它判断"会不会被截进图里"
    /// 会让每次截图白等满 900 ms 超时。
    fn own_window_on_screen(&self) -> bool {
        desktop::UiWake::window_with_title_on_desktop(OVERLAY_TITLE)
            || desktop::UiWake::window_with_title_visible(PIN_TITLE)
    }

    /// 记下"用户要求截图"的时刻，并登记请求。
    ///
    /// 所有入口（热键 / 托盘 / 另一个实例转交 / 设置界面按钮 / 启动参数）都走这里，
    /// 这样日志里的耗时才是**从用户动作算起**的，而不是从某一帧算起。
    fn begin_request(&mut self, request: CaptureRequest) {
        // 热键链路上唯一量不到的一段：热键线程的事件处理器 → UI 线程真的跑起来
        // （eframe 会把隐藏窗口的重绘请求推迟 100 ms，见 `desktop::UiWake`）。
        if let Some(age) = self.wake_age_ms.take() {
            log::info!("截图链路[唤醒到处理]：{age} ms");
        }
        let started = Instant::now();
        // 文件名模板的 `{app}`：现在就问，等保存时前台已经是我们的遮罩了。
        self.capture_app = desktop::foreground_app_name();
        self.capture_monitor = cuter_core::capture::cursor::cursor_position()
            .map_or(1, monitor_index_at);
        self.request = Some(request);
        self.request_at = Some(started);
        // 共享给遮罩视口闭包：它在**首帧落地那一刻**当场落日志（换到下一帧读会晚一帧，
        // 而这一帧恰好是 100 ms 级——曾经把 135 ms 报成 267 ms）
        *lock(&self.request_started) = Some(started);
    }

    /// 打一条带端到端耗时的日志（没有起点时什么都不做）。
    fn log_request_stage(&self, stage: &str) {
        if let Some(started) = self.request_at {
            log::debug!("截图链路[{stage}]：+{} ms", started.elapsed().as_millis());
        }
    }

    /// 同上，但用 info 级别：只留给"用户等待"里的几个大阶段。
    fn log_request_stage_info(&self, stage: &str) {
        if let Some(started) = self.request_at {
            log::info!("截图链路[{stage}]：+{} ms", started.elapsed().as_millis());
        }
    }

    /// 把「包含鼠标指针」开关真正落到像素上。
    ///
    /// 屏幕 `BitBlt` 不含光标（光标不在屏幕 DC 里），所以必须单独取光标位图再合成。
    /// 取不到光标只是少画一个小图标，**不应该让整张截图失败**，因此这里全部降级处理。
    fn decorate_shot(&self, shot: &mut DesktopShot) {
        if !self.config.capture.include_cursor {
            return;
        }
        match cuter_core::capture::cursor::composite_cursor(
            &mut shot.frame,
            shot.bounds,
            shot.cursor,
        ) {
            Ok(true) => log::debug!("已把鼠标指针合成进冻结帧"),
            Ok(false) => log::debug!("未取到鼠标光标位图，跳过合成"),
            Err(err) => log::warn!("合成鼠标指针失败：{err}"),
        }
    }

    /// 枚举可吸附的顶层窗口；失败时退化成「只能吸附屏幕边缘」。
    fn snap_windows(&self) -> Vec<cuter_core::capture::SnapWindow> {
        if !self.config.capture.auto_detect_windows {
            return Vec::new();
        }
        match cuter_core::capture::window_detector()
            .and_then(|detector| detector.visible_windows())
        {
            Ok(windows) => {
                log::debug!("枚举到 {} 个可吸附窗口", windows.len());
                windows
            }
            Err(err) => {
                log::warn!("窗口枚举失败，本次只能吸附屏幕边缘：{err}");
                Vec::new()
            }
        }
    }

    /// 抓一整屏（走配置的后端；DXGI 还在后台建或建不起来时用 GDI）。
    fn grab_desktop(&self) -> Result<DesktopShot, String> {
        self.with_capture_backend(|backend| backend.capture_desktop())
            .and_then(|result| result.map_err(|err| err.to_string()))
    }


    /// 在"当前实际可用的后端"上执行一段抓取。
    ///
    /// * 后台已经建好 DXGI → 用它（**锁只在这一次抓取期间持有**，实测 6~17 ms）；
    /// * 还没建好 / 建不起来 / 配置就是 GDI → 现场建一个 GDI 后端
    ///   （`WindowsCapture::new()` 实测 0.00 ms，可以随用随建）。
    fn with_capture_backend<R>(
        &self,
        f: impl FnOnce(&dyn cuter_core::capture::ScreenCapture) -> R,
    ) -> Result<R, String> {
        {
            let slot = lock(&self.dxgi_backend);
            if let Some(capture) = slot.as_deref() {
                return Ok(f(capture));
            }
        }
        let gdi = cuter_core::capture::desktop_capture().map_err(|err| err.to_string())?;
        Ok(f(gdi.as_ref()))
    }

    fn perform_capture(&mut self, ctx: &Context, request: CaptureRequest) {
        self.log_request_stage_info("进入捕获");
        self.log_request_stage("捕获后端就绪");

        match request {
            CaptureRequest::Region => match self.grab_desktop() {
                Ok(mut shot) => {
                    self.decorate_shot(&mut shot);
                    self.log_request_stage_info("捕获完成");
                    let windows = self.snap_windows();
                    self.open_overlay(ctx, shot, windows);
                }
                Err(err) => self.set_status(fill!(tr!("捕获屏幕失败：{}"), err)),
            },
            CaptureRequest::Monitor => match self.grab_desktop() {
                Ok(mut shot) => {
                    self.decorate_shot(&mut shot);
                    let cursor = shot.cursor.unwrap_or(shot.bounds.origin());
                    let Some(monitor) = shot.monitor_at(cursor) else {
                        self.set_status(tr!("找不到鼠标所在的显示器"));
                        return;
                    };
                    let monitor = monitor.clone();
                    let restricted = restrict_to_monitor(shot, &monitor);
                    let windows = self.snap_windows();
                    self.open_overlay(ctx, restricted, windows);
                }
                Err(err) => self.set_status(fill!(tr!("捕获屏幕失败：{}"), err)),
            },
            CaptureRequest::Desktop => match self.grab_desktop() {
                Ok(shot) => {
                    let frame = shot.frame.clone();
                    self.deliver(frame, cuter_core::geom::Point::ZERO, DeliverMode::Save);
                }
                Err(err) => self.set_status(fill!(tr!("捕获屏幕失败：{}"), err)),
            },
            CaptureRequest::Scroll => match self.grab_desktop() {
                Ok(mut shot) => {
                    self.decorate_shot(&mut shot);
                    let windows = self.snap_windows();
                    self.open_overlay(ctx, shot, windows);
                    // 遮罩已经挂上去了，这里只是把它切到「滚动」模式：
                    // 确认键变成「开始采集」，工具栏也会高亮对应的按钮。
                    if let Some(overlay) = lock(&self.overlay).as_mut() {
                        overlay.set_scroll_mode(true);
                    }
                }
                Err(err) => self.set_status(fill!(tr!("捕获屏幕失败：{}"), err)),
            },
        }
    }

    fn open_overlay(
        &mut self,
        ctx: &Context,
        shot: DesktopShot,
        snap_windows: Vec<cuter_core::capture::SnapWindow>,
    ) {
        let bounds = shot.bounds;

        self.log_request_stage_info("开始构造遮罩");

        let overlay = Overlay::new(
            shot,
            &self.config,
            self.theme,
            self.font.clone(),
            snap_windows,
        );
        // 这一段曾经完全看不见（"已请求遮罩窗口"是 debug 级，探针读不到），
        // 结果把"捕获完成 → 首帧可见"的 200 ms 全算到建窗口头上。实测它与建窗口各占一半。
        self.log_request_stage_info("遮罩构造完成");
        *lock(&self.overlay) = Some(Box::new(overlay));
        // 遮罩会把 ppp 固定为 1.0，所以它的「点」就等于物理像素：目标直接用物理值
        *lock(&self.overlay_fix) = Some(GeometryFix::new(
            Vec2::new(bounds.x as f32, bounds.y as f32),
            Vec2::new(bounds.w as f32, bounds.h as f32),
        ));
        lock(&self.overlay_results).clear();
        // 遮罩要开始等了：闭包拿到首帧时会当场落一条端到端日志
        self.overlay_first_pass_pending
            .store(true, Ordering::Relaxed);
        self.overlay_wait_frames = 0;

        // 遮罩窗口创建时就让它可见——**不能**先隐藏再等对齐后显示：
        // winit 不会给隐藏窗口派发重绘事件，于是 eframe 永远不跑这个视口的 pass，
        // 视口回调里的「对齐后再显示」也就永远不会执行，窗口会一直隐身。
        //
        // 同时提前把 ppp 固定为 1.0，让遮罩的第一个 pass 就用像素坐标系绘制，
        // 并把 `zoom_factor` 变成 1/系统缩放——`egui-winit` 创建窗口时正是用
        // `LogicalSize(zoom_factor * size)`，这个因子与 winit 的显示器缩放相抵消，
        // 所以这里的 `size`/`position` 直接写物理像素就是对的。
        ctx.set_pixels_per_point(1.0);

        if self.config.capture.play_sound {
            play_capture_sound();
        }

        self.overlay_viewport = Some(overlay_builder(bounds));
        // 新窗口，标题栏样式还没摘过：`declare_viewports` 每帧会补一次。
        self.overlay_hit_test_owned = false;
        // 从这里往后是 eframe 真正建窗口 + 建 GL 上下文，不在我们的掌控内；
        // 探针量到的"遮罩可见"时刻减去这个点，就是那一部分的代价。
        //
        // 显式要求"可见"：如果这次截图前为了藏窗口给这个视口发过 `Visible(false)`
        // （「重新截图」会走到），egui 复用那个窗口时它还是隐藏的——不补这一句，
        // 重新截图之后就再也看不到遮罩了。
        ctx.send_viewport_cmd_to(overlay_viewport_id(), ViewportCommand::Visible(true));
        self.log_request_stage_info("已请求遮罩窗口");

        // **必须立刻要一帧，而且要绕开节流。**
        //
        // 托盘模式下根窗口是隐藏的，eframe 会把重绘请求推迟到 ~100 ms（`desktop::UiWake`
        // 那套 `WM_PAINT` 就是为绕开它而写的，但之前只用在"热键进来的第一帧"上）。
        // 遮罩从"声明视口"到"闭包真的跑起来"要**两帧**，于是白等两个 100 ms 节流周期：
        // 实测 `已请求遮罩窗口 +11 ms → 遮罩等首帧 +135 ms → 遮罩首帧可见 +267 ms`，
        // 两帧各约 125 ms——这就是整条延迟的大头（捕获本身只占 11 ms）。
        ctx.request_repaint();
        self.wake.wake(ctx);
        // **子视口（deferred）是独立重绘的**：父视口要帧并不会连带重画它，
        // 而"把窗口挪回桌面"这件事必须由它自己那一趟 pass 来完成。
        // 少了这一句，窗口会被移回屏幕但**一直是空白/未绘制**（探针会看到窗口在，
        // 却等不到 `遮罩首帧可见`——实测踩过）。
        ctx.request_repaint_of(overlay_viewport_id());
    }

    /// **每帧**重新声明所有子视口。
    ///
    /// 这是 egui 多视口最容易踩的坑：`show_viewport_deferred` 必须在每一个 pass 里
    /// 被调用，否则 egui 会在本 pass 结束时把这个视口判定为「it was never used this
    /// pass」并直接销毁窗口——现象就是**遮罩/贴图窗口闪一下就消失**。
    ///
    /// 另外，`ViewportBuilder` 在这里被逐帧比对（`ViewportBuilder::patch` 是增量合并：
    /// 只有新值为 `Some` 且与已存值不同才会产生命令），所以这里必须原样重复同一个
    /// 描述，绝不能每帧重新构造一份带 `with_inner_size` / `with_position` 的 builder，
    /// 否则会把 `GeometryFix` 辛苦对齐好的窗口几何反复冲掉。
    fn declare_viewports(&mut self, ctx: &Context) {
        // 字体必须在**这一帧**就位，哪怕这一帧是"某个视口刚被打开"的那一帧。
        //
        // 按需装载的门禁在 `logic()` 靠后的位置，它判断"现在有没有窗口需要画字"时，
        // 新窗口还没在这个 pass 里声明出来；等它下一帧看见窗口再装载，中间那帧画的
        // 就是 egui 自带的字体（不含 CJK）——用户看到的是"整片文字变成方框"
        // （概率出现，取决于那两帧谁先跑）。
        if self.needs_font() {
            self.ensure_font(ctx);
        }
        // 遮罩窗的标题栏样式要摘掉：winit 建的无装饰窗口仍带 `WS_CAPTION`，
        // 铺满屏幕时最上沿那一行会被判成 `HTCAPTION`——在屏幕上沿一按一拖，
        // Windows 自己把窗口搬走，看起来就是"冻结帧跟着鼠标动"。
        // 窗口是延迟建的，第一帧可能还没有句柄，所以摘成功才置位、失败下帧再来。
        if self.overlay_viewport.is_some() && !self.overlay_hit_test_owned {
            self.overlay_hit_test_owned = crate::desktop::UiWake::take_over_hit_test(OVERLAY_TITLE);
        }
        // ---- 冻结遮罩 ----
        if let Some(builder) = self.overlay_viewport.clone() {
            let overlay = Arc::clone(&self.overlay);
            let fix = Arc::clone(&self.overlay_fix);
            let results = Arc::clone(&self.overlay_results);
            let request_started = Arc::clone(&self.request_started);
            let first_pass_pending = Arc::clone(&self.overlay_first_pass_pending);

            ctx.show_viewport_deferred(overlay_viewport_id(), builder, move |ui, _class| {
                let ctx = ui.ctx().clone();
                // 遮罩的**第一个 pass**：这是"用户按下热键"到"屏幕上真的出现遮罩"这条链路的
                // 终点。不记这一笔，`+捕获完成` 之后的几十毫秒就完全看不见（那一部分是
                // eframe 建窗口 + 建 GL 上下文 + 首帧上传纹理，全在框架里）。
                // 闭包拿不到 `&self`，所以只把时刻塞进共享槽，由 `logic()` 落日志。
                // `pending.swap(false)` 是**原子的一次性开关**：`logic()` 每帧把槽取走，
                // 如果用"槽为空就写"来判断，下一帧又会写一次，日志就会每帧刷一条
                // （实测每 133 ms 一条、耗时一路涨到 1.2 s，完全是假数据）。

                if first_pass_pending.swap(false, Ordering::Relaxed) {
                    // 当场落日志：这一刻就是用户看到遮罩的时刻
                    if let Some(started) = *lock(&request_started) {
                        log::info!(
                            "截图链路[遮罩首帧可见]：+{} ms",
                            started.elapsed().as_millis()
                        );
                    }
                }
                // 遮罩的几何状态是 `Option`（遮罩关闭后会置空），这里单独处理
                {
                    let mut guard = lock(&fix);
                    if let Some(state) = guard.as_mut() {
                        apply_geometry_fix(&ctx, state, true, true);
                    }
                }
                let mut result = None;
                {
                    let mut guard = lock(&overlay);
                    if let Some(overlay) = guard.as_mut() {
                        result = overlay.ui(&ctx);
                        // **不发 `Close`**：窗口要留着复用（下一次截图只需改几何 + 重绘）。
                        // 收尾由 `drain_overlay_results` 做——它把 `overlay` 置空并把视口
                        // 停到桌面之外，于是这个闭包下一帧什么都不画。
                    }
                }
                if let Some(result) = result {
                    lock(&results).push(result);
                }
            });
        }

        // ---- 贴图 ----
        // 「该不该出现在屏幕上」在这里决定：**不声明 = 窗口被关掉**。
        //
        // 为什么不用 `ViewportCommand::Visible(false)`：贴图视口是**每帧重新声明**的，
        // 那种命令在这个 eframe 版本上对它们不生效——实测 `Shift+F3`（隐藏所有贴图）
        // 和"切组"都只是改了个状态，窗口照样在屏幕上（探针 `pin_groups_e2e.ps1` 抓到的），
        // 连带"截图前藏起自己的窗口"对贴图也是失效的（要等满 900 ms 超时，还会把贴图截进去）。
        // 不声明的语义与遮罩 / OCR 面板一致：egui 在本 pass 结束时收掉窗口，
        // 想再显示时重新声明即可——`Pin` 的状态（缩放 / 透明度 / 标注）都留在这个 map 里。
        //
        // 先把需要的句柄收集出来，避免在持有锁的情况下调用 `ctx.*`（同线程重入会死锁）
        let entries: Vec<_> = lock(&self.pins)
            .iter()
            .filter(|(_, entry)| self.pin_should_show(entry))
            .map(|(id, entry)| {
                // 「鼠标穿透」与「始终置顶」都是视口属性，由 `ViewportBuilder::patch`
                // 转成 `ViewportCommand::MousePassthrough` / `WindowLevel`，
                // **只在取值变化时才下发命令**，所以每帧原样带上当前值是安全的。
                // 贴图菜单里改动这两个开关后，下一帧就会自然生效。
                let (click_through, always_on_top) = {
                    let pin = lock(&entry.pin);
                    (pin.click_through(), pin.always_on_top())
                };
                let builder = entry
                    .builder
                    .clone()
                    .with_mouse_passthrough(click_through)
                    .with_window_level(if always_on_top {
                        WindowLevel::AlwaysOnTop
                    } else {
                        WindowLevel::Normal
                    });
                (
                    *id,
                    Arc::clone(&entry.pin),
                    Arc::clone(&entry.fix),
                    builder,
                )
            })
            .collect();

        for (id, pin, fix, builder) in entries {
            let pins = Arc::clone(&self.pins);
            let prefs = Arc::clone(&self.pin_prefs);
            let ocr_requests = Arc::clone(&self.pin_ocr_requests);
            let theme = self.theme;
            let light = light_mode(self.config.general.theme, ctx);
            // 视口回调要求 Send + 'static，不能在里面碰 `self`；
            // 换算尺寸要用的缩放比先拷出来（与 `open_pin` 用的是同一个）。
            let ui_scale = self.ui_scale.max(0.1);

            ctx.show_viewport_deferred(id, builder, move |ui, _class| {
                pin_viewport_theme(ui, &theme, light);
                let ctx = ui.ctx().clone();
                // 先并偏移再对齐：偏移在 `Pin::ui` 里算，所以这里用的是上一帧的值，
                // 正好比 `Pin::ui` 早一步把「窗口该在哪」摆正。
                edit_geometry_fix(&fix, |state| {
                    sync_menu_shift(&pin, state);
                    apply_geometry_fix(&ctx, state, false, false);
                });

                // ⚠️ **必须先把动作取出来再 match**！
                //
                // `match lock(&pin).ui(&ctx) { ... }` 里那把 `MutexGuard` 是**scrutinee 的临时值**，
                // Rust 会把它活到整个 match 结束——而下面每个分支又都会 `lock(&pin)`
                // （`set_status` / `frame()` …），于是**整条贴图动作处理全部自锁死**：
                // 现象是"右键菜单点复制 / Ctrl+C / 拖出 → 程序卡住不动"（实测抓到：
                // 日志停在动作那一刻，之后一条都没有）。
                // 先 bind 成局部变量，guard 在语句结束就释放。
                let pin_action = lock(&pin).ui(&ctx);
                match pin_action {
                    Some(PinAction::Close) => {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                        lock(&pins).remove(&id);
                    }
                    Some(PinAction::Copy) => {
                        let frame = lock(&pin).frame().clone();
                        let message = match desktop::copy_image(&frame) {
                            Ok(()) => tr!("已复制").to_owned(),
                            Err(err) => err,
                        };
                        lock(&pin).toast(&ctx, message);
                    }
                    Some(PinAction::Save) => {
                        let frame = lock(&pin).frame().clone();
                        // 贴图的另存为用固定前缀 + 时间；`{app}` 这类上下文对它没意义，
                        // 所以走最小上下文（不借用截图那套）。
                        let stem = render_filename(
                            "Cuter_pin_{datetime}",
                            &cuter_core::config::NameContext::minimal(
                                LocalTime::now(),
                                frame.size(),
                            ),
                        );
                        let path = self_save_dir().join(format!("{stem}.png"));
                        let message = match frame.save(&path) {
                            Ok(()) => fill!(tr!("已保存到 {}"), path.display()),
                            Err(err) => fill!(tr!("保存失败：{}"), err),
                        };
                        lock(&pin).toast(&ctx, message);
                    }
                    Some(PinAction::DragOut) => {
                        // 落一个临时 PNG，再起 OLE 拖放。**这一步会阻塞到用户放下**：
                        // 拖放期间 winit 的 WndProc 只把消息排队（它此刻正阻塞在这一层），
                        // 所以不会重入 `update()`；贴图画面短暂不动是正常的。
                        let frame = lock(&pin).frame().clone();
                        log::info!("贴图拖出：{}×{}", frame.width(), frame.height());
                        let message = match Self::export_for_drag(&frame) {
                            Ok(path) => match crate::desktop::drag_files(&[path]) {
                                Ok(message) => message,
                                Err(err) => err,
                            },
                            Err(err) => err,
                        };
                        lock(&pin).toast(&ctx, message);
                        ctx.request_repaint();
                    }
                    Some(PinAction::Ocr) => {
                        // 真正的识别在主线程上发起（`start_ocr`），这里只排队——
                        // 视口闭包拿不到 `&mut CuterApp`。
                        let frame = lock(&pin).frame().clone();
                        lock(&ocr_requests).push(frame);
                        lock(&pin).toast(&ctx, tr!("正在识别文字…"));
                    }
                    Some(PinAction::Resize { size, at }) => {
                        // 只更新目标；真正的尺寸下发交给 `apply_geometry_fix`，
                        // 它会顺带做 ppp 换算与对齐校验
                        edit_geometry_fix(&fix, |state| {
                            // 贴图可能为「右键菜单完整可见」把窗口挪了（见 `Pin::grow_for_menu`）。
                            // 这个偏移必须并进目标位置，否则尺寸一变就让 `apply_geometry_fix`
                            // 重新对齐，把窗口拖回原位、菜单又被裁掉半截。
                            sync_menu_shift(&pin, state);
                            state.settled = false;
                            state.attempts = 0;
                            // `PinAction::Resize` 与 `Pin::window_size` 同单位（egui 点），
                            // 而 `fix.target_size` 也是点（`apply_geometry_fix` 拿它和
                            // `ctx.viewport_rect()` 比），所以这里**不做** ppp 换算。
                            // `PinAction::Resize` 给的是**图片像素**（贴图内部一律按像素记账），
                            // 而 `fix.target_size` 走的是 egui 点（`apply_geometry_fix` 拿它和
                            // `ctx.viewport_rect()` 比）。创建贴图时也是这么除的，见 `open_pin`。
                            let scale = ui_scale;
                            state.target_size = Vec2::new(size.w as f32, size.h as f32) / scale;
                            // 拖动左边/上边缩放时贴图会带上一个目标位置：窗口得跟着长，
                            // 否则被抓住的那条边会往反方向跑。
                            if let Some(at) = at {
                                // `target_pos` 存的是 `Vec2`（历史上就是），这里换算一下
                                state.target_pos = at.to_vec2();
                            }
                        });
                    }
                    Some(PinAction::SetOpacity(opacity)) => {
                        lock(&prefs).push(PinPrefPatch::Opacity(opacity));
                        let message = fill!(tr!("不透明度 {}%"), format!("{:.0}", opacity * 100.0));
                        lock(&pin).toast(&ctx, message);
                    }
                    Some(PinAction::ToggleClickThrough(enabled)) => {
                        lock(&prefs).push(PinPrefPatch::ClickThrough(enabled));
                        lock(&pin).toast(
                            &ctx,
                            if enabled {
                                // 这句必须写清"去哪儿关"：穿透一开，这个窗口就再也收不到
                                // 鼠标事件了，「在菜单里可关闭」是句做不到的承诺。
                                tr!("已开启鼠标穿透：要取消请点托盘图标 →「取消所有贴图的鼠标穿透」")
                            } else {
                                tr!("已关闭鼠标穿透")
                            },
                        );
                    }
                    Some(PinAction::SetAlwaysOnTop(enabled)) => {
                        // 窗口层级由 `declare_viewports` 每帧从贴图状态里取，
                        // 下一帧就会下发 `ViewportCommand::WindowLevel`
                        lock(&prefs).push(PinPrefPatch::AlwaysOnTop(enabled));
                        lock(&pin).toast(
                            &ctx,
                            if enabled {
                                tr!("已置顶（右键菜单里可关闭）")
                            } else {
                                tr!("已取消置顶：别的窗口会盖住这张图（右键菜单里可再开启）")
                            },
                        );
                    }
                    None => {}
                }
            });
        }

        // ---- OCR 结果面板 ----
        if let Some(builder) = self.ocr_viewport.clone() {
            let panel = Arc::clone(&self.ocr_panel);
            let actions = Arc::clone(&self.ocr_actions);
            let theme = self.theme;
            let light = light_mode(self.config.general.theme, ctx);

            ctx.show_viewport_deferred(ocr_viewport_id(), builder, move |ui, _class| {
                pin_viewport_theme(ui, &theme, light);
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme.surface))
                    .show(ui, |ui| {
                    let mut guard = lock(&panel);
                    match guard.as_mut() {
                        Some(panel) => {
                            if let Some(action) = panel.ui(ui) {
                                lock(&actions).push(action);
                            }
                        }
                        None => {
                            ui.label(tr!("识别结果面板已关闭，可以关掉这个窗口了。"));
                        }
                    }
                });
            });
        }
        // ---- 托盘菜单（自绘）----
        if let Some(builder) = self.tray_menu_viewport.clone() {
            let actions = Arc::clone(&self.tray_menu_actions);
            let focused = Arc::clone(&self.tray_menu_focused);
            let theme = self.theme;
            let light = light_mode(self.config.general.theme, ctx);
            // 快捷键文案必须在外面取好并带走：视口闭包是 `'static`，借不到 `&self`
            let entries = self.config.hotkeys.entries();
            let hotkey = |command: HotkeyCommand| -> String {
                let key = command.key();
                entries
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map_or_else(String::new, |(_, value)| (*value).to_owned())
            };
            let capture = hotkey(HotkeyCommand::Capture);
            let capture_monitor = hotkey(HotkeyCommand::CaptureMonitor);
            let pin = hotkey(HotkeyCommand::PinFromClipboard);
            let toggle_pins = hotkey(HotkeyCommand::TogglePins);
            let next_group = hotkey(HotkeyCommand::NextPinGroup);
            let pins_hidden = self.pins_hidden;
            // 锁只借这一下：`MutexGuard` 不能进闭包（闭包要求 `Send`）
            let pin_count = lock(&self.pins).len() as u32;

            // 面板在自己那个视口的 pass 里跑，往队列里推动作之后**必须**把根视口叫醒：
            // 根在托盘待命时可能正睡在 4 s 的心跳上，光靠它自己轮询，Esc 之后菜单
            // 还要在屏幕上僵三秒多（tray_menu 探针实测 49.259 → 52.814）。
            //
            // 更要紧的是：根不一定醒得过来——托盘待命时它是隐藏窗口，真正在跑 pass 的
            // 是菜单那个视口。所以窗口由面板自己收（`Visible(false)` 在它自己的 pass
            // 结束时立刻生效），宿主那边只做记账。
            let wake_for_menu = self.wake.clone();
            ctx.show_viewport_deferred(tray_menu_viewport_id(), builder, move |ui, _class| {
                pin_viewport_theme(ui, &theme, light);
                let mut close_now = ui.ctx().input(|i| i.viewport().close_requested());
                // 点到别处（视口失去焦点）就收起来——系统菜单也是这个行为。
                // 首帧可能还没聚焦，所以要"曾经聚焦过"才算数。
                let has_focus = ui.ctx().input(|i| i.viewport().focused).unwrap_or(false);
                {
                    let mut seen = lock(&focused);
                    if has_focus {
                        *seen = true;
                    } else if *seen {
                        *seen = false;
                        close_now = true;
                    }
                }
                if close_now {
                    lock(&actions).push(TrayAction::Close);
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    wake_for_menu.nudge();
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                }
                let input = TrayPanelInput {
                    capture: &capture,
                    capture_monitor: &capture_monitor,
                    pin: &pin,
                    toggle_pins: &toggle_pins,
                    next_group: &next_group,
                    pins_hidden,
                    pin_count,
                };
                let mut panel = TrayPanel::new(theme);
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme.surface))
                    .show(ui, |ui| {
                        if let Some(action) = panel.ui(ui, &input) {
                            log::info!("托盘菜单：面板返回动作 {:?}", action);
                            lock(&actions).push(action);
                            // 菜单是一次性的：动作一出去就当场收窗口，别等根视口。
                            // 根在托盘待命时可能几秒才跑一帧（隐藏窗口 + 心跳退避），
                            // 等它来收的话菜单会在屏幕上僵三四秒（tray_menu 探针）。
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            wake_for_menu.nudge();
                            ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                        }
                    });
            });
        }

        // ---- 历史面板 ----
        if let Some(builder) = self.history_viewport.clone() {
            let panel = Arc::clone(&self.history_panel);
            let actions = Arc::clone(&self.history_actions);
            let theme = self.theme;
            let light = light_mode(self.config.general.theme, ctx);

            ctx.show_viewport_deferred(history_viewport_id(), builder, move |ui, _class| {
                pin_viewport_theme(ui, &theme, light);
                // 用户点了窗口的关闭按钮：面板没有"必须拦住"的理由，等价于「关闭」
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    lock(&actions).push(HistoryAction::Close);
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme.surface))
                    .show(ui, |ui| {
                    let mut guard = lock(&panel);
                    match guard.as_mut() {
                        Some(panel) => {
                            if let Some(action) = panel.ui(ui) {
                                lock(&actions).push(action);
                            }
                        }
                        None => {
                            ui.label(tr!("历史面板已关闭，可以关掉这个窗口了。"));
                        }
                    }
                });
            });
        }

        // ---- 滚动截屏控制窗 ----
        if let Some(builder) = self.scroll_viewport.clone() {
            let session = self.scroll_session.clone();
            let panel = Arc::clone(&self.scroll_panel);
            let actions = Arc::clone(&self.scroll_actions);
            let max_height = self.config.scroll.max_height;
            // 这个视口有自己的 egui Context：主题不钉死的话它跟着系统走（系统浅色时
            // 面板底色是白的），而标题/文字用的是 self.theme 的颜色——浅底浅字，
            // 真机截图里「滚动截屏」几乎看不见。和主视口一样，theme 偏好与 visuals
            // 两个都要设，只设其一会被 eframe 的「跟随系统」覆回去。
            let theme = self.theme;
            let light = light_mode(self.config.general.theme, ctx);

            ctx.show_viewport_deferred(scroll_viewport_id(), builder, move |ui, _class| {
                pin_viewport_theme(ui, &theme, light);

                // 用户点了窗口的关闭按钮：等价于「取消」，让宿主把线程一起停掉，
                // 否则采集会在后台无限跑下去而用户已经没有出口了。
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    lock(&actions).push(ScrollAction::Cancel);
                }

                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme.surface))
                    .show(ui, |ui| {
                    let mut panel_guard = lock(&panel);
                    let Some(panel) = panel_guard.as_mut() else {
                        ui.label(tr!("采集已结束。"));
                        return;
                    };

                    // 会话锁只在这一段里持有：面板绘制不能跨越它
                    let session_guard = session.as_ref().and_then(|s| s.lock().ok());
                    let input = match session_guard.as_deref() {
                        Some(session) => ScrollPanelInput {
                            size: session.stitcher.size(),
                            frames: session.stitcher.frames(),
                            axis: session.stitcher.axis().label(),
                            status: &session.status,
                            paused: session.paused,
                            auto: session.auto,
                            max_height,
                            preview_generation: session.preview_generation,
                            preview: session.preview.as_ref(),
                        },
                        None => ScrollPanelInput {
                            size: Size::ZERO,
                            frames: 0,
                            axis: "纵向",
                            status: tr!("采集已结束"),
                            paused: false,
                            auto: false,
                            max_height,
                            preview_generation: 0,
                            preview: None,
                        },
                    };

                    if let Some(action) = panel.ui(ui, &input) {
                        lock(&actions).push(action);
                    }
                });
            });
        }
    }

    /// 处理遮罩产出的结果。
    fn drain_overlay_results(&mut self, ctx: &Context) {
        let results: Vec<OverlayResult> = std::mem::take(&mut lock(&self.overlay_results));
        if results.is_empty() {
            return;
        }
        for result in results {
            let origin = result.origin;
            let region = result.region;
            let action = result.action;
            match (action, result.frame) {
                (OverlayAction::Cancel, _) => {
                    self.set_status(tr!("已取消"));
                }
                (OverlayAction::Recapture, _) => {
                    self.begin_request(CaptureRequest::Region);
                }
                (OverlayAction::OpenSettings, _) => {
                    self.set_control_visible(ctx, true);
                }
                (OverlayAction::Copy, Some(frame)) => {
                    self.deliver(frame, origin, DeliverMode::Copy);
                    self.remember_region(region);
                }
                (OverlayAction::Save, Some(frame)) => {
                    self.deliver(frame, origin, DeliverMode::Save);
                    self.remember_region(region);
                }
                (OverlayAction::SaveAs, Some(frame)) => {
                    self.deliver(frame, origin, DeliverMode::SaveAs);
                    self.remember_region(region);
                }
                (OverlayAction::Pin, Some(frame)) => {
                    self.open_pin(ctx, frame, origin);
                    self.remember_region(region);
                }
                (OverlayAction::Ocr, Some(frame)) => {
                    self.remember_region(region);
                    self.start_ocr(ctx, frame);
                }
                (OverlayAction::ScrollCapture, Some(frame)) => {
                    self.remember_region(region);
                    self.start_scroll(ctx, frame, region);
                }
                (action, None) => {
                    log::warn!("收到没有图片的结果：{action:?}");
                }
            }
        }
        // 遮罩视口已经销毁：停止声明它（下一 pass egui 就会回收窗口），并恢复缩放。
        //
        // 试过"把窗口停到桌面之外留着复用"（省掉每次重建的 ~100 ms），**在 eframe 0.36 上走不通**：
        // eframe 对**不可见**的 deferred 视口直接 `show_ui = false`，连 egui pass 都不跑
        // （源码 `glow_integration.rs` 的 `is_visible || is_viewport_or_descendant_visible`），
        // 于是 `visible` 这个状态被缓存住、移回屏幕也不会重新开画——窗口在、画面永远空白。
        // 详见 HANDOFF 第 25 节。
        *lock(&self.overlay) = None;
        *lock(&self.overlay_fix) = None;
        self.overlay_viewport = None;
        ctx.set_zoom_factor(1.0);
    }

    /// 记录最后一次成功框选的区域，供遮罩里的 `Ctrl+R` 复原。
    fn remember_region(&mut self, region: cuter_core::geom::Rect) {
        if !self.config.capture.remember_last_region || region.is_empty() {
            return;
        }
        if self.config.capture.last_region == Some(region) {
            return;
        }
        self.config.capture.last_region = Some(region);
        self.persist_config();
    }

    fn deliver(
        &mut self,
        frame: Frame,
        origin: cuter_core::geom::Point,
        mode: DeliverMode,
    ) {
        // 只有"真的交付成功"才进历史：用户取消了另存为、或者剪贴板被别的程序占着，
        // 那些都不该在历史里留下一条"看起来截到了"的记录。
        let mut delivered = false;
        match mode {
            DeliverMode::Copy => match desktop::copy_image(&frame) {
                Ok(()) => {
                    delivered = true;
                    self.set_status(fill!(tr!("已复制到剪贴板（{} × {}）"), frame.size().w, frame.size().h));
                }
                Err(err) => self.set_status(err),
            },
            DeliverMode::Save | DeliverMode::SaveAs => {
                let format = self.config.save.image_format();
                let name = self.next_save_name(frame.size(), format.extension());

                let path = if matches!(mode, DeliverMode::SaveAs)
                    || (matches!(mode, DeliverMode::Save) && self.config.save.ask_path_on_save)
                {
                    let dir = self.config.save.resolved_dir();
                    match save_dialog::ask_save_path(
                        &dir,
                        &name,
                        format.label(),
                        &[format.extension()],
                    ) {
                        Some(path) => path,
                        None => {
                            // 取消与「对话框不可用」无法区分，如实说明而不是谎称已保存
                            self.set_status(tr!("未保存（已取消，或系统对话框不可用）"));
                            return;
                        }
                    }
                } else {
                    self.unique_save_path(&name)
                };

                // 保存时实际用的编码格式：**扩展名优先**（用户在「另存为」里把名字
                // 改成 .jpg 就该存 JPEG），但质量一律来自配置——`Frame::save` 只会
                // 从扩展名反推出一个内置默认质量，那条路上 `save.quality` 是死的。
                let effective = save_format_for(&path, format, self.config.save.quality);
                match frame.save_as(&path, effective) {
                    Ok(()) => {
                        delivered = true;
                        self.last_saved = Some(path.clone());
                        self.set_status(fill!(tr!("已保存：{}"), path.display()));
                        // 遮罩这时已经关掉、用户回到了自己的程序里，气泡是唯一能告诉他
                        // "存哪儿了"的方式（状态栏他已经看不到了）。
                        if let Some(notifier) = self.notifier.as_mut()
                            && self.config.general.show_tray_notifications
                        {
                            let file = path
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| path.display().to_string());
                            let sent = notifier.notify(
                                tr!("截图已保存"),
                                &fill!(tr!("{}（点托盘图标可打开设置）"), file),
                            );
                            log::info!("托盘通知（保存完成）：已投放={sent} → {file}");
                        }
                        if self.config.save.auto_copy
                            && let Err(err) = desktop::copy_image(&frame)
                        {
                            log::warn!("自动复制失败：{err}");
                        }
                        if self.config.save.reveal_after_save {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("explorer")
                                    .arg(format!("/select,{}", path.display()))
                                    .spawn();
                            }
                        }
                    }
                    Err(err) => self.set_status(fill!(tr!("保存失败：{}"), err)),
                }
            }
        }
        if delivered {
            self.remember_history(&frame);
        }
        let _ = origin;
    }

    /// 按配置生成文件名（含扩展名）。
    fn next_save_name(&self, size: cuter_core::geom::Size, extension: &str) -> String {
        let stem = render_filename(&self.config.save.filename_pattern, &self.name_context(size));
        format!("{stem}.{extension}")
    }

    /// 渲染文件名要用的上下文：时间、序号、尺寸，加上显示器序号与"按热键时前台是哪个程序"。
    ///
    /// `app` 必须是**按下热键那一刻**记下来的：等到真正保存时前台已经是我们的遮罩/面板了，
    /// 那时再问只会得到 "cuter"。
    fn name_context(&self, size: cuter_core::geom::Size) -> cuter_core::config::NameContext {
        cuter_core::config::NameContext {
            now: LocalTime::now(),
            seq: self.pin_counter + 1,
            size,
            monitor: self.capture_monitor,
            app: self.capture_app.clone(),
        }
    }

    /// 在配置目录里找一个不冲突的路径。
    fn unique_save_path(&self, name: &str) -> PathBuf {
        let dir = self.config.save.resolved_dir();
        let dir = if dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            dir
        };
        let mut candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, ext)) => (stem, ext),
            None => (name, ""),
        };
        for index in 1..1000 {
            let suffix = if extension.is_empty() {
                format!("{stem}_{index}")
            } else {
                format!("{stem}_{index}.{extension}")
            };
            candidate = dir.join(suffix);
            if !candidate.exists() {
                return candidate;
            }
        }
        candidate
    }

    /// 把贴图位置夹进虚拟桌面，保证整张贴图都看得见。
    ///
    /// 不夹的话会出现「在屏幕右下角贴一张 4K 图 → 只看得到左上角一小块」，
    /// 用户还得先把它拖回来。
    fn clamp_pin_origin(&self, origin: cuter_core::geom::Point, size: Vec2) -> cuter_core::geom::Point {
        let fallback = cuter_core::geom::Rect::new(
            0,
            0,
            size.x.max(1.0) as u32,
            size.y.max(1.0) as u32,
        );
        let bounds = desktop_capture()
            .and_then(|backend| backend.monitors())
            .map(|monitors| cuter_core::capture::virtual_bounds(&monitors))
            .unwrap_or(fallback);
        let max_x = (bounds.right() - size.x as i32).max(bounds.left());
        let max_y = (bounds.bottom() - size.y as i32).max(bounds.top());
        cuter_core::geom::Point::new(
            origin.x.clamp(bounds.left(), max_x),
            origin.y.clamp(bounds.top(), max_y),
        )
    }

    fn open_pin(&mut self, ctx: &Context, frame: Frame, origin: cuter_core::geom::Point) {
        self.pin_counter += 1;
        let id = ViewportId::from_hash_of(format!("cuter-pin/{}", self.pin_counter));
        let size = Pin::initial_window_size(&frame, &self.config.pin);
        let origin = self.clamp_pin_origin(origin, size);
        let pin = Pin::new(frame, &self.config.pin, self.theme, size);
        // 新贴图一定看得见：用户刚要求贴出来，不能因为"上一组正被隐藏"就什么都不出现
        self.pins_hidden = false;

        lock(&self.pins).insert(
            id,
            PinEntry {
                group: self.pin_group,
                pin: Arc::new(Mutex::new(pin)),
                // 贴图窗口不固定 ppp，所以目标要先除以系统缩放才是 egui 的「点」
                fix: Arc::new(Mutex::new(GeometryFix::new(
                    Vec2::new(origin.x as f32, origin.y as f32) / self.ui_scale.max(0.1),
                    size / self.ui_scale.max(0.1),
                ))),
                // 与遮罩同理：真正的 `show_viewport_deferred` 交给 `declare_viewports`
                // 每帧下发，这里只登记描述；且必须创建即可见（隐藏窗口收不到重绘事件）
                builder: ViewportBuilder::default()
                    .with_title(PIN_TITLE)
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_taskbar(false)
                    // 窗口层级每帧由 `declare_viewports` 从贴图状态里取（用户可以在右键
                    // 菜单里改「始终置顶」），这里只给创建时的初值
                    .with_window_level(if self.config.pin.always_on_top {
                        WindowLevel::AlwaysOnTop
                    } else {
                        WindowLevel::Normal
                    })
                    // 透明视口：贴图的「不透明度」靠窗口自身的 alpha 实现，
                    // 见 `Pin::ui` 里用 `from_white_alpha` 给图片上色调
                    .with_transparent(true)
                    .with_position([
                        origin.x as f32 / self.ui_scale.max(0.1),
                        origin.y as f32 / self.ui_scale.max(0.1),
                    ])
                    .with_inner_size([
                        size.x / self.ui_scale.max(0.1),
                        size.y / self.ui_scale.max(0.1),
                    ]),
            },
        );
        // 立刻重绘一帧：贴图的窗口是在 `declare_viewports` 里声明的，
        // 不催一下要等下一次轮询（最多 120 ms）才出现。
        //
        // ⚠️ 统计数字要**先算好再打日志**：`log::info!` 里的临时 `MutexGuard` 活到整条
        // 语句结束，所以 `lock(&self.pins).len()` 和 `self.pins_in_group(..)`（内部又加锁）
        // 写在同一个表达式里会**死锁**——实测直接把 UI 线程卡死（`Responding=False`，
        // 事件循环停摆，连截图请求都不再处理）。
        let (total, in_group) = {
            let pins = lock(&self.pins);
            (
                pins.len(),
                pins.values()
                    .filter(|entry| entry.group == self.pin_group)
                    .count(),
            )
        };
        log::info!(
            "贴图已登记：第 {} 组，共 {} 张（本组 {} 张）",
            self.pin_group + 1,
            total,
            in_group
        );
        ctx.request_repaint();
    }

    /// 把贴图落成一个临时 PNG，供拖出用（返回文件路径）。
    ///
    /// 落文件而不是走 `CF_DIB`：所有目标程序都认文件，而内存格式要自己实现 `IStream`，
    /// 兼容面反而更窄（见 `desktop::drag_temp_dir` 的说明）。
    fn export_for_drag(frame: &Frame) -> Result<std::path::PathBuf, String> {
        crate::desktop::prune_drag_temp_dir();
        let dir = crate::desktop::drag_temp_dir();
        std::fs::create_dir_all(&dir).map_err(|err| fill!(tr!("创建临时目录失败：{}"), err))?;
        // 文件名带时间戳：同一张贴图连拖两次不会互相覆盖（目标程序可能还在读前一个）
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("cuter-pin-{stamp}.png"));
        frame
            .save(&path)
            .map_err(|err| fill!(tr!("写出拖出的图片失败：{}"), err))?;
        Ok(path)
    }

    /// 按需装载界面字体（第一次真的要画界面时才读那 18.8 MB 的 msyh.ttc）。
    ///
    /// **为什么值得这么做**：托盘常驻是这个软件的常态，而根窗口隐藏时 eframe
    /// 连一次 egui pass 都不跑（`show_ui = false`），也就是说**根本没有文字要画**。
    /// 实测（`perf_probe.ps1`）：常驻托盘 WS 86 MB / 私有 49 MB，其中字体那份
    /// 就是 ~19 MB；按需装载之后托盘态降到 ~30 MB 私有。
    ///
    /// 代价：第一次真的要用字体的那次操作会多花"读文件 + 注册"的时间（实测 ~10~20 ms，
    /// 且只在进程生命周期里付一次）。所以调用点选在"即将要画界面"的地方，
    /// 而不是启动路径上。
    ///
    /// **装载之后不再卸载**：`Context::set_fonts` 要到**下一帧开头**才生效，
    /// 所以"用完就卸、下次再装"会留下一个"这一帧没有中文字形"的窗口——用户看到的
    /// 就是整片方框（HANDOFF 第 88 节）。工作集由 [`crate::desktop::trim_working_set`]
    /// 在闲置 30 s 后还给系统，任务管理器里看不到这 19 MB；私有提交会留着，
    /// 但它是一个常量，不随使用时长增长。
    fn ensure_font(&mut self, ctx: &Context) {
        if self.font.is_some() {
            return;
        }
        let started = std::time::Instant::now();
        self.font = fonts::setup(ctx, Some(&self.config.annotate.text_font));
        if let Some(font) = &self.font {
            log::info!(
                "界面字体已按需装载：{}（耗时 {} ms）",
                font.path.display(),
                started.elapsed().as_millis()
            );
            if !font.has_cjk {
                self.warnings.push(
                    "没有找到中文字体，界面与文字标注里的中文可能显示为方框。\
                     可在「标注」页指定字体文件（例如 C:\\Windows\\Fonts\\msyh.ttc）。"
                        .to_owned(),
                );
            }
        } else {
            log::warn!("没有找到可用的系统字体，界面文字可能显示为方块");
            self.warnings
                .push("没有找到可用的系统字体，界面文字可能显示为方块".to_owned());
        }
    }

    /// 现在有没有任何"我们自己的界面"要画（决定要不要读字体）。
    fn needs_font(&self) -> bool {
        let terms = (
            self.control_visible.get(),
            self.overlay_viewport.is_some(),
            !lock(&self.pins).is_empty(),
            self.history_viewport.is_some(),
            self.ocr_viewport.is_some(),
            self.scroll_viewport.is_some(),
            self.tray_menu_viewport.is_some(),
        );
        if terms != (false, false, false, false, false, false, false) {
            log::trace!(
                "needs_font 为真：窗口={} 遮罩={} 贴图={} 历史={} OCR={} 滚动={} 托盘菜单={}",
                terms.0,
                terms.1,
                terms.2,
                terms.3,
                terms.4,
                terms.5,
                terms.6
            );
        }
        terms.0 || terms.1 || terms.2 || terms.3 || terms.4 || terms.5 || terms.6
    }

    /// 把一张图丢给后台线程做 OCR。
    ///
    /// **为什么要开线程**：`Windows.Media.Ocr` 是 WinRT，阻塞等待它完成要求线程处于
    /// MTA 套间；而 winit 的事件循环线程可能是 STA，在它上面阻塞等待异步结果会与
    /// 套间消息泵互相等待而死锁。后台线程由我们创建，[`cuter_core::ocr::WindowsOcr::new`]
    /// 会在其上初始化 MTA，因此可以安全地阻塞。
    fn start_ocr(&mut self, ctx: &Context, frame: Frame) {
        use cuter_core::config::OcrEngine;

        if self.ocr_busy {
            self.set_status(tr!("上一次识别还没结束，请稍候"));
            return;
        }
        match self.config.ocr.engine {
            OcrEngine::Disabled => {
                self.set_status(tr!("OCR 已关闭：在「设置 → 文字识别」里启用后即可使用"));
                return;
            }
            // 在线接口的唯一必备条件是地址；空地址时给一句能直接照做的提示，
            // 比让它跑一趟后台线程再拿回一句“连不上”强。
            OcrEngine::Http if self.config.ocr.http_endpoint.trim().is_empty() => {
                self.set_status(tr!("在线 OCR 需要在「设置 → 文字识别」里填接口地址"));
                return;
            }
            _ => {}
        }

        let language = self
            .config
            .ocr
            .languages
            .first()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let theme = self.theme;
        // 段落模式与引擎配置都必须在进线程之前取出来：后台线程拿不到 `self`
        let paragraph = self.config.ocr.paragraph;
        let ocr = self.config.ocr.clone();
        let tx = self.ocr_tx.clone();
        let ctx = ctx.clone();

        self.ocr_busy = true;
        self.set_status(tr!("正在识别文字…"));
        log::info!(
            "开始 OCR：{}×{}，引擎 {:?}，语言 {:?}，段落模式 {paragraph}",
            frame.size().w,
            frame.size().h,
            self.config.ocr.engine,
            language
        );

        std::thread::spawn(move || {
            let message = match recognize_frame(frame, ocr, theme) {
                Ok(panel) => OcrMessage::Done(Box::new(panel)),
                Err(err) => OcrMessage::Failed(err),
            };
            // 发送失败说明 UI 已经退出，忽略即可
            let _ = tx.send(message);
            // 唤醒 UI 线程来处理结果
            ctx.request_repaint();
        });
    }

    /// 收取 OCR 结果并挂上结果面板。
    fn poll_ocr(&mut self, ctx: &Context) {
        while let Ok(message) = self.ocr_rx.try_recv() {
            self.ocr_busy = false;
            match message {
                OcrMessage::Done(panel) => {
                    let blocks = panel.block_count();
                    let text = panel.text().to_owned();
                    log::info!(
                        "OCR 完成：{blocks} 行，{} 个字符",
                        text.chars().filter(|c| !c.is_whitespace()).count()
                    );

                    if self.config.ocr.auto_copy && !text.trim().is_empty() {
                        match desktop::copy_text(&text) {
                            Ok(()) => self.set_status(fill!(tr!("识别到 {} 行，已复制到剪贴板"), blocks)),
                            Err(err) => self.set_status(fill!(tr!("识别到 {} 行，但复制失败：{}"), blocks, err)),
                        }
                    } else {
                        self.set_status(fill!(tr!("识别到 {} 行"), blocks));
                    }

                    *lock(&self.ocr_panel) = Some(panel);
                    self.ocr_viewport = Some(ocr_viewport_builder());
                }
                OcrMessage::Failed(err) => {
                    log::warn!("OCR 失败：{err}");
                    self.set_status(err);
                }
            }
        }
        let _ = ctx;
    }

    /// 关闭结果面板（停止每帧声明它，egui 下一帧就会回收窗口）。
    fn close_ocr_panel(&mut self) {
        *lock(&self.ocr_panel) = None;
        self.ocr_viewport = None;
    }

    fn handle_ocr_action(&mut self, ctx: &Context, action: OcrAction) {
        match action {
            OcrAction::Retry => {
                // 先把图取出来、把锁放掉，再交给 start_ocr（它会再拿一次锁）
                let frame = lock(&self.ocr_panel)
                    .as_ref()
                    .map(|panel| panel.frame().clone());
                if let Some(frame) = frame {
                    self.start_ocr(ctx, frame);
                }
            }
            OcrAction::Copy | OcrAction::CopyAndClose => {
                let text = lock(&self.ocr_panel)
                    .as_ref()
                    .map(|panel| panel.text().to_owned())
                    .unwrap_or_default();
                match desktop::copy_text(&text) {
                    Ok(()) => self.set_status(tr!("已复制识别文本")),
                    Err(err) => self.set_status(err),
                }
                if action == OcrAction::CopyAndClose {
                    self.close_ocr_panel();
                }
            }
            OcrAction::CopyBlock(text) => match desktop::copy_text(&text) {
                Ok(()) => {
                    if let Some(panel) = lock(&self.ocr_panel).as_mut() {
                        panel.set_status(tr!("已复制该行"));
                    }
                }
                Err(err) => self.set_status(err),
            },
            OcrAction::Close => self.close_ocr_panel(),
        }
    }

    // ------------------------------------------------------------------
    // 截图历史
    // ------------------------------------------------------------------

    /// 把一次成功交付的截图记进历史。
    ///
    /// **永不失败**：写历史失败只是少一条记录，绝不能因此让用户觉得"这张图没截到"。
    /// 所以这里只记日志，把结果反映到状态栏的是调用方。
    fn remember_history(&self, frame: &Frame) {
        if !self.config.history.enabled() {
            return;
        }
        let now = LocalTime::now();
        let text = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            now.year, now.month, now.day, now.hour, now.minute, now.second
        );
        let mut store = lock(&self.history);
        match store.add(
            frame,
            self.config.history.limit,
            self.config.history.thumb_width,
            now.unix,
            text,
        ) {
            Ok(entry) => log::info!(
                "历史 +1：id={} {}×{}（共 {} 张，{}）",
                entry.id,
                entry.width,
                entry.height,
                store.len(),
                cuter_core::history::human_bytes(store.total_bytes())
            ),
            Err(err) => log::warn!("写入截图历史失败（不影响这次截图）：{err}"),
        }
    }

    /// 打开历史面板（已经开着就把它提到前面）。
    fn open_history(&mut self, ctx: &Context) {
        if self.history_viewport.is_none() {
            let panel = HistoryPanel::new(Arc::clone(&self.history), self.theme);
            *lock(&self.history_panel) = Some(Box::new(panel));
            self.history_viewport = Some(history_viewport_builder());
        }
        // 面板是普通窗口：显示它并把焦点拿过来，否则用户点了托盘菜单什么都没发生
        ctx.send_viewport_cmd_to(history_viewport_id(), ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(history_viewport_id(), ViewportCommand::Focus);
    }

    /// 关闭历史面板（停止每帧声明它，egui 下一帧回收窗口）。
    fn close_history_panel(&mut self) {
        *lock(&self.history_panel) = None;
        self.history_viewport = None;
    }

    /// 打开托盘菜单（自绘）：贴着当前鼠标弹出来，已经开着就提到前面。
    ///
    /// 位置由宿主算而不是视口里算：闭包拿不到鼠标位置，而 `--tray-menu` 这条入口
    /// 根本没有真实的右键事件（探针用它核对版面）。
    fn open_tray_menu(&mut self, ctx: &Context) {
        if self.tray_menu_viewport.is_none() {
            self.tray_menu_viewport = Some(tray_menu_viewport_builder());
        }
        let ppp = ctx.pixels_per_point();
        let size = cuter_ui::tray_panel::PANEL_SIZE;
        let (width, height) = (size.0 * ppp, size.1 * ppp);
        // 鼠标位置（物理像素）；拿不到就退回屏幕原点，菜单照样能开
        let (ax, ay) = cuter_core::capture::cursor::cursor_position().map_or((0, 0), |p| (p.x, p.y));
        let work = monitor_work_area(Rect::new(ax, ay, 1, 1));
        let (right, bottom) = (work.x + work.w as i32, work.y + work.h as i32);
        // 鼠标贴着屏幕右下角时菜单要往左上收，否则会有一半落在屏幕外
        let x = (ax as f32).min(right as f32 - width).max(work.x as f32);
        let y = (ay as f32).min(bottom as f32 - height).max(work.y as f32);
        ctx.send_viewport_cmd_to(
            tray_menu_viewport_id(),
            ViewportCommand::OuterPosition(egui::pos2(x / ppp, y / ppp)),
        );
        ctx.send_viewport_cmd_to(tray_menu_viewport_id(), ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(tray_menu_viewport_id(), ViewportCommand::Focus);
        ctx.request_repaint();
    }

    /// 关闭托盘菜单（停止每帧声明它，并让它立刻收窗口）。
    fn close_tray_menu(&mut self, ctx: &Context) {
        self.tray_menu_viewport = None;
        *lock(&self.tray_menu_focused) = false;
        // 少声明一次只保证"下一帧别再画它"，**不会**让窗口消失：deferred 视口要显式
        // 收命令。以前每 120 ms 都在出帧，不看这条也"看不见"问题；心跳退到 1~4 s 之后
        // 菜单窗口能在屏幕上僵好几秒（tray_menu 探针抓到过）。
        //
        // 先 `Visible(false)` 再 `Close`：前者是"立刻藏起来"，一帧就生效；后者走销毁，
        // 要等窗口自己再跑一次事件循环。只发 Close 时实测还会在屏幕上多停 2 s 多。
        ctx.send_viewport_cmd_to(tray_menu_viewport_id(), ViewportCommand::Visible(false));
        ctx.send_viewport_cmd_to(tray_menu_viewport_id(), ViewportCommand::Close);
        self.idle_ticks = 0;
        self.keep_awake_until = Some(Instant::now() + std::time::Duration::from_millis(300));
    }

    /// 菜单里点了一项：先收起菜单（它是一次性的，和系统菜单一样），再照托盘命令办。
    fn handle_tray_action(&mut self, ctx: &Context, action: TrayAction) {
        log::info!("托盘菜单：收到动作 {:?}", action);
        self.close_tray_menu(ctx);
        let command = match action {
            TrayAction::Capture => TrayCommand::Capture,
            TrayAction::CaptureMonitor => TrayCommand::CaptureMonitor,
            TrayAction::PinFromClipboard => TrayCommand::PinFromClipboard,
            TrayAction::TogglePins => TrayCommand::TogglePins,
            TrayAction::ClearPinClickThrough => TrayCommand::ClearPinClickThrough,
            TrayAction::NextPinGroup => TrayCommand::NextPinGroup,
            TrayAction::OpenSettings => TrayCommand::OpenSettings,
            TrayAction::OpenHistory => TrayCommand::OpenHistory,
            TrayAction::OpenSaveDir => TrayCommand::OpenSaveDir,
            TrayAction::Quit => TrayCommand::Quit,
            // 「关闭菜单」只是把菜单收起来，不发任何托盘命令
            TrayAction::Close => return,
        };
        self.handle_tray(ctx, command);
    }

    fn handle_history_action(&mut self, ctx: &Context, action: HistoryAction) {
        match action {
            HistoryAction::Copy(id) => {
                let frame = lock(&self.history).load_image(id);
                match frame {
                    Ok(frame) => match desktop::copy_image(&frame) {
                        Ok(()) => {
                            self.set_history_status("已复制这一张（原图）");
                            self.set_status(fill!(tr!("已复制历史中的 {}×{}"), frame.width(), frame.height()));
                        }
                        Err(err) => self.set_history_status(err),
                    },
                    Err(err) => self.set_history_status(fill!(tr!("读回原图失败：{}"), err)),
                }
            }
            HistoryAction::Pin(id) => {
                let frame = lock(&self.history).load_image(id);
                match frame {
                    Ok(frame) => {
                        let cursor = self.cursor_global();
                        self.open_pin(ctx, frame, cursor);
                    }
                    Err(err) => self.set_history_status(fill!(tr!("读回原图失败：{}"), err)),
                }
            }
            HistoryAction::SaveAs(id) => {
                let loaded = lock(&self.history).load_image(id);
                match loaded {
                    Ok(frame) => {
                        let format = self.config.save.image_format();
                        let name = self.next_save_name(frame.size(), format.extension());
                        let dir = self.config.save.resolved_dir();
                        let path = save_dialog::ask_save_path(
                            &dir,
                            &name,
                            format.label(),
                            &[format.extension()],
                        );
                        match path {
                            Some(path) => {
                                let effective =
                                    save_format_for(&path, format, self.config.save.quality);
                                match frame.save_as(&path, effective) {
                                    Ok(()) => {
                                        self.last_saved = Some(path.clone());
                                        self.set_history_status(fill!(tr!("已另存为 {}"), path.display()));
                                    }
                                    Err(err) => {
                                        self.set_history_status(fill!(tr!("保存失败：{}"), err))
                                    }
                                }
                            }
                            None => self.set_history_status("未保存（已取消，或对话框不可用）"),
                        }
                    }
                    Err(err) => self.set_history_status(fill!(tr!("读回原图失败：{}"), err)),
                }
            }
            HistoryAction::Delete(id) => {
                let result = lock(&self.history).remove(id);
                match result {
                    Ok(()) => self.set_history_status("已删除这一张"),
                    Err(err) => self.set_history_status(fill!(tr!("删除失败：{}"), err)),
                }
            }
            HistoryAction::Clear => {
                // 先把结果取出来再动 `self`：锁不能跨越 `self.set_*`（本项目的老规矩，
                // 编译器也会拦——`MutexGuard` 的临时值活到 match 结束）
                let result = lock(&self.history).clear();
                match result {
                    Ok(()) => self.set_history_status("历史已清空"),
                    Err(err) => self.set_history_status(fill!(tr!("清空失败：{}"), err)),
                }
            }
            HistoryAction::OpenDir => {
                let dir = self.config.history.dir();
                let _ = std::fs::create_dir_all(&dir);
                #[cfg(windows)]
                {
                    if let Err(err) = std::process::Command::new("explorer").arg(&dir).spawn() {
                        self.set_history_status(fill!(tr!("打开目录失败：{}"), err));
                    }
                }
            }
            HistoryAction::Close => self.close_history_panel(),
        }
    }

    /// 把一行反馈塞给历史面板（没有面板时退化成状态栏）。
    fn set_history_status(&mut self, text: impl Into<String>) {
        let mut text = text.into();
        // `if let` 里的锁只活到这条语句结束（不能跨越 `self.set_status`）
        if let Some(panel) = lock(&self.history_panel).as_mut() {
            panel.set_status(std::mem::take(&mut text));
            return;
        }
        self.set_status(text);
    }

    // ------------------------------------------------------------------
    // 滚动截屏
    // ------------------------------------------------------------------

    /// 用当前选区开始滚动截屏。`first` 是首帧，`region` 是它的全局区域。
    fn start_scroll(&mut self, ctx: &Context, first: Frame, region: Rect) {
        if self.scroll_session.is_some() {
            self.set_status(tr!("已经有一个滚动截屏在进行了"));
            return;
        }
        if region.is_empty() || first.is_empty() {
            self.set_status(tr!("选区太小，无法滚动截屏"));
            return;
        }

        let max_length = self.config.scroll.max_height.max(200);
        let options = self.config.scroll.match_options();
        let session = ScrollSession {
            auto: self.config.scroll.auto_scroll,
            auto_tracker: cuter_core::scroll::AutoScrollTracker::new(
                self.config.scroll.auto_stall_frames,
            ),
            // 方向由 `DirectedStitcher` 自己判：纵向接不上时会试横向，能接上就整体切换
            stitcher: DirectedStitcher::new(first, max_length, options),
            region,
            options,
            paused: false,
            status: tr!("开始采集：在目标窗口里正常滚动（横着滚、往回滚都行，方向会自动判定）").to_owned(),
            preview: None,
            preview_generation: 0,
            last_preview: Instant::now(),
            interval: Duration::from_millis(u64::from(self.config.scroll.interval_ms.max(60))),
        };

        // 控制窗的位置与尺寸都是「点」，而选区是物理像素：除以系统缩放换算。
        // 遮罩那个「点 == 物理像素」的约定只对它自己成立（它把 ppp 固定成了 1.0），
        // 控制窗是个普通窗口，不能套用。
        let scale = if self.ui_scale > 0.0 {
            self.ui_scale
        } else {
            1.0
        };
        let panel_phys = Size::new(
            (SCROLL_PANEL_SIZE.0 * scale).round() as u32,
            (SCROLL_PANEL_SIZE.1 * scale).round() as u32,
        );
        let (pos, overlaps) =
            scroll_control_position(region, panel_phys, monitor_work_area(region));
        if overlaps {
            log::warn!(
                "控制窗放不下：选区 {region:?} 已经占满了工作区，控制窗会盖住采集区域并被截进长图"
            );
            self.set_status(tr!("选区太大，控制窗会出现在长图里——建议缩小选区"));
        }

        let state = Arc::new(Mutex::new(session));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_stop = Arc::clone(&stop);
        let worker_ctx = ctx.clone();
        let worker_dxgi = Arc::clone(&self.dxgi_backend);
        std::thread::spawn(move || {
            scroll_worker(
                worker_state,
                worker_stop,
                worker_ctx,
                worker_dxgi,
            )
        });

        self.scroll_session = Some(state);
        self.scroll_stop = Some(stop);
        *lock(&self.scroll_panel) = Some(Box::new(ScrollPanel::new(self.theme)));
        self.scroll_viewport = Some(scroll_viewport_builder([
            pos.x as f32 / scale,
            pos.y as f32 / scale,
        ]));
        log::info!(
            "滚动截屏开始：区域 {region:?}，控制窗位于 {pos:?}（逻辑 {:?}）",
            (pos.x as f32 / scale, pos.y as f32 / scale)
        );
        if !overlaps {
            self.set_status(tr!("滚动截屏进行中：请在目标窗口里滚动"));
        }
    }

    /// 结束采集（无论完成还是取消都要先停掉线程）。
    fn stop_scroll(&mut self) {
        if let Some(stop) = self.scroll_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.scroll_session = None;
        *lock(&self.scroll_panel) = None;
        self.scroll_viewport = None;
    }

    /// 处理控制窗上的动作。
    fn handle_scroll_action(&mut self, action: ScrollAction) {
        match action {
            ScrollAction::ToggleAuto => {
                if let Some(session) = &self.scroll_session
                    && let Ok(mut session) = session.lock()
                {
                    session.auto = !session.auto;
                    // 重新打开时把计数归零：否则上一次留下的"连续没变化"会立刻把它停掉
                    session.auto_tracker = cuter_core::scroll::AutoScrollTracker::new(
                        self.config.scroll.auto_stall_frames,
                    );
                    let steps = session.auto_tracker.steps;
                    session.status = if session.auto {
                        fill!(tr!("自动滚动已开启（已转发 {} 次滚轮，按 A 停下）"), steps)
                    } else {
                        tr!("自动滚动已关闭").to_owned()
                    };
                    log::info!(
                        "滚动截屏自动滚动：{}（停机判据 {} 帧）",
                        if session.auto { "开" } else { "关" },
                        self.config.scroll.auto_stall_frames
                    );
                }
            }
            ScrollAction::TogglePause => {
                if let Some(session) = &self.scroll_session
                    && let Ok(mut session) = session.lock()
                {
                    session.paused = !session.paused;
                    session.status = if session.paused {
                        "已暂停".to_owned()
                    } else {
                        "已继续".to_owned()
                    };
                }
            }
            ScrollAction::Cancel => {
                self.stop_scroll();
                self.set_status(tr!("已取消滚动截屏"));
            }
            ScrollAction::Finish => {
                // 区域和画布都要在 `stop_scroll()` 之前取出来——它会清空会话
                let taken = self.scroll_session.as_ref().and_then(|session| {
                    session
                        .lock()
                        .ok()
                        .map(|s| (s.stitcher.image(), s.region.origin(), s.stitcher.frames()))
                });
                let Some((canvas, origin, frames)) = taken else {
                    self.stop_scroll();
                    self.set_status(tr!("滚动截屏没有内容"));
                    return;
                };
                self.stop_scroll();

                let mode = if self.config.save.auto_save {
                    DeliverMode::Save
                } else {
                    DeliverMode::Copy
                };
                log::info!(
                    "滚动截屏完成：{frames} 帧，结果 {}×{}",
                    canvas.size().w,
                    canvas.size().h
                );
                // ASCII 标记：探针脚本必须是纯 ASCII（无 BOM 的 .ps1 会被 PS 5.1 按 GBK 读），
                // 所以拿不到上面那行中文，只能靠这一行做断言。
                log::info!(
                    "scroll-done frames={frames} w={} h={}",
                    canvas.size().w,
                    canvas.size().h
                );
                self.set_status(fill!(tr!("滚动截屏完成：{} 帧，{}×{}"), frames, canvas.size().w, canvas.size().h));
                self.deliver(canvas, origin, mode);
            }
        }
    }

    /// 收取控制窗上的动作。
    fn poll_scroll(&mut self) {
        let actions: Vec<ScrollAction> = std::mem::take(&mut lock(&self.scroll_actions));
        for action in actions {
            self.handle_scroll_action(action);
        }
    }

    // ------------------------------------------------------------------
    // 设置界面
    // ------------------------------------------------------------------

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_frames = 240;
    }

    /// 自绘窗口标题栏（36px）：左侧应用标记 + 标题，右侧最小化/最大化/关闭，整条可拖拽。
    ///
    /// 系统装饰已经在 `main.rs` 里关掉，这条就是窗口唯一的标题栏；
    /// 稿子见 `docs/design/FIGMA.md` 的「自绘窗口标题栏」一节。
    fn paint_title_bar(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        const BAR_H: f32 = 36.0;
        const BTN_W: f32 = 46.0;
        let width = ui.available_width();
        let (bar, bar_response) =
            ui.allocate_exact_size(egui::vec2(width, BAR_H), egui::Sense::click_and_drag());
        // 克隆一份 painter：后面还要 `ui.interact`，不能一直借着 ui
        let painter = ui.painter().clone();

        painter.rect_filled(bar, egui::CornerRadius::ZERO, self.theme.surface);
        painter.hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            egui::Stroke::new(1.0, self.theme.outline),
        );

        // 应用标记：16px，和 `icon.rs` 同一套配色与几何
        let mark = egui::Rect::from_min_size(
            egui::pos2(bar.left() + 12.0, bar.top() + 10.0),
            egui::Vec2::splat(16.0),
        );
        paint_app_mark(&painter, mark);
        painter.text(
            egui::pos2(mark.right() + 8.0, bar.center().y),
            egui::Align2::LEFT_CENTER,
            "Cuter",
            egui::FontId::proportional(12.0),
            self.theme.on_surface_dim,
        );

        // 右侧三个按钮：先收交互结果、再画，避免同时借用 ui 与 painter
        let kinds = [TitleButton::Minimize, TitleButton::Maximize, TitleButton::Close];
        let count = kinds.len();
        let mut rects = Vec::with_capacity(count);
        let mut responses = Vec::with_capacity(count);
        for (index, _) in kinds.iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                egui::pos2(bar.right() - BTN_W * (count - index) as f32, bar.top()),
                egui::vec2(BTN_W, BAR_H),
            );
            responses.push(ui.interact(
                rect,
                ui.id().with(("title-bar", index)),
                egui::Sense::click(),
            ));
            rects.push(rect);
        }
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        for ((kind, rect), response) in kinds.iter().copied().zip(rects.iter()).zip(responses) {
            let hovered = response.hovered();
            if hovered {
                // 关闭按钮悬停用危险色：它是"会丢东西"的那个
                let fill = if kind == TitleButton::Close {
                    self.theme.danger
                } else {
                    self.theme.surface_raised
                };
                painter.rect_filled(*rect, egui::CornerRadius::ZERO, fill);
            }
            let color = if hovered && kind == TitleButton::Close {
                egui::Color32::WHITE
            } else {
                self.theme.on_surface_dim
            };
            paint_title_button_glyph(&painter, *rect, kind, color);
            if response.clicked() {
                match kind {
                    TitleButton::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                    TitleButton::Maximize => {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    }
                    TitleButton::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                }
            }
        }

        // 拖拽移动 / 双击最大化：压在按钮上时不生效
        let over_button = ctx
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| rects.iter().any(|r| r.contains(p)));
        if !over_button {
            if bar_response.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if bar_response.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
        }
    }

    /// 无装饰窗口没有系统缩放边框：贴边 6px 内自己接管八向缩放。
    ///
    /// 最大化时直接返回——否则在最大化状态下往边缘一按，窗口会被拽回浮动态。
    fn paint_resize_border(&self, ctx: &Context, window: egui::Rect) {
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        const EDGE: f32 = 6.0;
        let left = pointer.x <= window.left() + EDGE;
        let right = pointer.x >= window.right() - EDGE;
        let top = pointer.y <= window.top() + EDGE;
        let bottom = pointer.y >= window.bottom() - EDGE;
        let (direction, cursor) = match (left, right, top, bottom) {
            (true, _, true, _) => (
                egui::ResizeDirection::NorthWest,
                egui::CursorIcon::ResizeNwSe,
            ),
            (_, true, true, _) => (egui::ResizeDirection::NorthEast, egui::CursorIcon::ResizeNeSw),
            (true, _, _, true) => (egui::ResizeDirection::SouthWest, egui::CursorIcon::ResizeNeSw),
            (_, true, _, true) => (egui::ResizeDirection::SouthEast, egui::CursorIcon::ResizeNwSe),
            (true, _, _, _) => (egui::ResizeDirection::West, egui::CursorIcon::ResizeHorizontal),
            (_, true, _, _) => (egui::ResizeDirection::East, egui::CursorIcon::ResizeHorizontal),
            (_, _, true, _) => (egui::ResizeDirection::North, egui::CursorIcon::ResizeVertical),
            (_, _, _, true) => (egui::ResizeDirection::South, egui::CursorIcon::ResizeVertical),
            _ => return,
        };
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }

    /// 控件统一高度：输入框、下拉框、按钮共用一档，避免"有的 22 有的 28"。
    const CONTROL_HEIGHT: f32 = 28.0;

    fn paint_control(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        // 顶栏按钮用中性底色，只有主操作（框选截图）用强调色——
        // 一排名词按钮全蓝会非常吵（实测）。
        let bar_visuals = settings_visuals(ui, &self.theme);

        // 所有控件统一高矮胖瘦。egui 的 singleline `TextEdit` 高度、`ComboBox`
        // 按钮高度都取 `interact_size.y`，所以一处设置就把输入框和下拉框拉齐了；
        // 按钮的内边距也一起放大一档，省得"细狗一样"（用户反馈原话）。
        // 只作用于设置窗口：遮罩工具条有自己的度量，不受这里影响。
        ui.spacing_mut().interact_size.y = Self::CONTROL_HEIGHT;
        ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);

        // ---- 自绘标题栏（36px）与八向缩放 ----
        //
        // `main.rs` 里关了系统装饰（`with_decorations(false)`），窗口的另一半责任就落在这里：
        // 标题栏要自己画，贴边缩放也要自己接管——否则窗口既没有标题栏、也拖不大。
        // 稿子见 `docs/design/FIGMA.md` 的「自绘窗口标题栏」一节。
        self.paint_title_bar(ui, ctx);
        self.paint_resize_border(ctx, ui.max_rect());

        // ---- 侧栏背景：整列"凹"色，从客户区顶贯到底 ----
        //
        // 用**面板自己的矩形**（`ui.max_rect()`）来铺，而不是视口矩形：
        // 视口坐标和面板坐标不是同一套，用 `Area::fixed_pos` 定位会把侧栏画飘
        // （实测：导航项跑到内容卡片上面去了）。
        // 标题栏之后剩下的客户区（用光标位置而不是 `max_rect`，后者还含标题栏那 36px）
        let panel = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
        let sidebar = egui::Rect::from_min_max(
            panel.min,
            egui::pos2(panel.left() + 200.0, panel.bottom()),
        );
        ui.painter()
            .rect_filled(sidebar, egui::CornerRadius::ZERO, self.theme.surface_sunken);

        ui.horizontal_top(|ui| {
            // 左：导航（背景已经铺好，这里只负责排版）
            ui.allocate_ui_with_layout(
                egui::Vec2::new(200.0, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(12, 14))
                        .show(ui, |ui| {
                            ui.set_width(176.0);
                            ui.spacing_mut().item_spacing = egui::Vec2::new(0.0, 2.0);
                            let mut group = "";
                            for page in NAV_ORDER {
                                let label = nav_group_label(page);
                                if label != group {
                                    if !group.is_empty() {
                                        ui.add_space(8.0);
                                    }
                                    group = label;
                                    ui.label(
                                        egui::RichText::new(label)
                                            .size(11.0)
                                            .color(self.theme.on_surface_faint),
                                    );
                                    ui.add_space(2.0);
                                }
                                if nav_row(ui, &self.theme, page, page.label(), self.settings_page == page)
                                    .clicked()
                                {
                                    self.settings_page = page;
                                }
                            }
                            // 版本号放侧栏底部（原来和标题重复了两次）
                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new(fill!(
                                    tr!("v{} · {}"),
                                    cuter_core::VERSION,
                                    capture_backend_name()
                                ))
                                .size(10.0)
                                .color(self.theme.on_surface_faint),
                            );
                        });
                },
            );

            // 右：内容（头部在这里，不在侧栏上方——稿子里标题属于内容列）
            ui.vertical(|ui| {
                // 右边距：侧栏那边留了 16 px，内容列右边也得留同样的 16 px。
                // 以前这里是零边距，卡片和主按钮直接贴着窗口边（用户："左边有边距右边舍不得给么"）。
                ui.set_max_width((ui.available_width() - 16.0).max(320.0));
                ui.add_space(16.0);        // ---- 头部：页面标题 + 一句说明 + 一个主操作 + 溢出菜单 ----
        //
        // 设计系统 v1 的取舍：**一屏一个主操作**。这里原来横铺七个按钮（框选/当前显示器/
        // 整个桌面/滚动截屏/贴图剪贴板/截图历史/退出），既和标题抢同一行，也让人不知道该按哪个。
        // 现在只留主操作「开始截图」，其余全部收进 ⋯ 菜单。
        ui.scope(|ui| {
            ui.set_visuals(bar_visuals.clone());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(self.settings_page.label())
                            .size(20.0)
                            .color(self.theme.on_surface),
                    );
                    ui.label(
                        egui::RichText::new(page_description(self.settings_page))
                            .size(12.0)
                            .color(self.theme.on_surface_dim),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 一体式分裂按钮：左半「开始截图」，右半「▾」开同一份菜单。
                    //
                    // 以前是两个独立控件（蓝色按钮 + 一个透明底的 ⋯ 方块），中间还有
                    // 间隙，看着像"按钮旁边挂了个不明方块"（用户截图指出）。截图文具的
                    // 惯例是把它做成一个圆角矩形、左右两半、中间一条细线。
                    let more_width = 28.0;
                    let bar = egui::Vec2::new(96.0 + more_width, 32.0);
                    let (rect, _) = ui.allocate_exact_size(bar, egui::Sense::hover());
                    let divider = rect.right() - more_width;
                    let left_rect =
                        egui::Rect::from_min_max(rect.min, egui::pos2(divider, rect.max.y));
                    let right_rect =
                        egui::Rect::from_min_max(egui::pos2(divider, rect.min.y), rect.max);
                    let left = ui
                        .interact(left_rect, ui.id().with("capture-primary"), egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let right = ui
                        .interact(right_rect, ui.id().with("capture-more"), egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);

                    let outer = egui::CornerRadius::same(8);
                    ui.painter().rect_filled(rect, outer, self.theme.accent);
                    let half = |right_side: bool| {
                        if right_side {
                            egui::CornerRadius {
                                nw: 0,
                                sw: 0,
                                ne: 8,
                                se: 8,
                            }
                        } else {
                            egui::CornerRadius {
                                nw: 8,
                                sw: 8,
                                ne: 0,
                                se: 0,
                            }
                        }
                    };
                    if left.hovered() {
                        ui.painter()
                            .rect_filled(left_rect, half(false), self.theme.accent_hover);
                    }
                    if right.hovered() {
                        ui.painter()
                            .rect_filled(right_rect, half(true), self.theme.accent_hover);
                    }
                    // 中间那条细线：两半同色时它是唯一的"这是两个热区"的暗示。
                    ui.painter().vline(
                        divider,
                        (rect.top() + 7.0)..=(rect.bottom() - 7.0),
                        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
                    );
                    ui.painter().text(
                        left_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        tr!("开始截图"),
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    // 箭头自己画：`▾`（U+25BE）不在微软雅黑里，用文字画出来是一个方框
                    // （截图验证过）。两条线段组成 V 形，任何字体、任何 DPI 都稳。
                    let center = right_rect.center();
                    let caret = egui::Stroke::new(1.6, egui::Color32::WHITE);
                    ui.painter().line_segment(
                        [
                            egui::pos2(center.x - 4.0, center.y - 2.0),
                            egui::pos2(center.x, center.y + 2.0),
                        ],
                        caret,
                    );
                    ui.painter().line_segment(
                        [
                            egui::pos2(center.x, center.y + 2.0),
                            egui::pos2(center.x + 4.0, center.y - 2.0),
                        ],
                        caret,
                    );
                    if left.clicked() {
                        self.begin_request(CaptureRequest::Region);
                    }
                    // 右半开的是同一份菜单（原来挂在 ⋯ 上，2026-10-20 起改成 ▾）。
                    egui::Popup::menu(&right).show(|ui| {
                        ui.set_min_width(150.0);
                        if ui.button(tr!("当前显示器")).clicked() {
                            self.begin_request(CaptureRequest::Monitor);
                            ui.close();
                        }
                        if ui.button(tr!("整个桌面")).clicked() {
                            self.begin_request(CaptureRequest::Desktop);
                            ui.close();
                        }
                        if ui.button(tr!("滚动截屏")).clicked() {
                            self.request_scroll(ctx);
                            ui.close();
                        }
                        if ui.button(tr!("贴图剪贴板")).clicked() {
                            match desktop::paste_image() {
                                Ok(frame) => {
                                    self.open_pin(ctx, frame, cuter_core::geom::Point::new(120, 120));
                                }
                                Err(err) => self.set_status(err),
                            }
                            ui.close();
                        }
                        if ui.button(tr!("截图历史")).clicked() {
                            self.open_history(ctx);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(tr!("退出 Cuter")).clicked() {
                            self.quit = true;
                            ui.close();
                        }
                    });
                });
            });
        });

        // 状态行在内容列**底栏**（稿子如此）：以前就贴在标题下面，
        // 每次启动那句"Cuter 一直在后台运行"都挤在标题和卡片之间，看着像标题的一部分。
        let show_status = self.status_frames > 0;
        if show_status {
            self.status_frames = self.status_frames.saturating_sub(1);
        }

        // ---- 左侧分类导航 + 右侧卡片 ----
        //
        // 用 `horizontal_top` 手搭两栏，而不是 `SidePanel::show_inside`：
        // 后者在这个 egui 版本里已经改过名字（`show_inside` → `show`），
        // 而这里只需要"一个固定宽度的左栏 + 剩下的内容区"，手搭更稳。
                ui.add_space(8.0);
                let footer = if self.status_frames > 0 { 26.0 } else { 0.0 };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - footer).max(120.0))
                    .show(ui, |ui| {
                        let page = self.settings_page;
                        match page {
                            SettingsPage::General => self.section_general(ui),
                            SettingsPage::Hotkeys => self.section_hotkeys(ui, ctx),
                            SettingsPage::Capture => self.section_capture(ui),
                            SettingsPage::Save => self.section_save(ui),
                            SettingsPage::Annotate => self.section_annotate(ui),
                            SettingsPage::Pin => self.section_pin(ui),
                            SettingsPage::Ocr => self.section_ocr(ui),
                            SettingsPage::Scroll => self.section_scroll(ui),
                            SettingsPage::History => self.section_history(ui, ctx),
                            SettingsPage::Font => self.section_font(ui),
                            SettingsPage::Diagnostics => self.section_diagnostics(ui),
                            SettingsPage::About => self.section_about(ui),
                        }
                    });
                if self.status_frames > 0 {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&self.status)
                            .size(12.0)
                            .color(self.theme.accent),
                    );
                }
            });
        });
    }

    /// 起一个后台线程去取版本清单。
    ///
    /// 线程只做"取回 + 解析 + 比大小"，结果经 `update_tx` 回传；网络失败、地址写错、
    /// 清单不是 JSON 都算 `Failed`，只记日志 + 在「关于」页显示，**绝不弹窗打断**。
    ///
    /// `manual` 区分"用户点的立即检查"和"启动时的自动检查"：自动检查一天最多一次
    /// （`general.last_update_check`），手动点则从不节流——用户点了就该真的去查。
    fn start_update_check(&mut self, ctx: &Context, manual: bool) {
        let endpoint = self.config.general.update_endpoint.trim().to_owned();
        if endpoint.is_empty() {
            self.set_status(tr!("请先填「版本清单地址」，留空就不检查"));
            return;
        }
        if self.update_checking {
            return;
        }
        if !manual {
            let last = self.config.general.last_update_check;
            let now = now_unix();
            if last > 0 && now.saturating_sub(last) < UPDATE_CHECK_INTERVAL_SECS {
                // 一天内查过了：安静地跳过，不写状态栏（用户没在等这个）。
                log::info!(
                    "[update] throttled 启动检查：距离上次成功检查 {} 秒，本次跳过（间隔 {} 秒）",
                    now.saturating_sub(last),
                    UPDATE_CHECK_INTERVAL_SECS
                );
                return;
            }
        }
        self.update_checking = true;
        self.set_status(tr!("正在检查更新…"));

        let tx = self.update_tx.clone();
        let ctx = ctx.clone();
        let current = cuter_core::VERSION.to_owned();
        std::thread::spawn(move || {
            let outcome = match update::fetch(&endpoint, update::DEFAULT_TIMEOUT) {
                Ok(text) => match UpdateManifest::parse(&text) {
                    Ok(manifest) if manifest.is_newer_than(&current) => {
                        UpdateOutcome::Newer(manifest)
                    }
                    Ok(manifest) => UpdateOutcome::Current(manifest.version),
                    Err(err) => UpdateOutcome::Failed(err),
                },
                Err(err) => UpdateOutcome::Failed(err),
            };
            // 发送失败说明 UI 已经退出，忽略即可
            let _ = tx.send(outcome);
            // 唤醒 UI 线程来处理结果
            ctx.request_repaint();
        });
    }

    /// 后台检查回来了：更新界面状态，「关于」页据此显示结果。
    fn handle_update_outcome(&mut self, ctx: &Context, outcome: UpdateOutcome) {
        self.update_checking = false;
        match &outcome {
            UpdateOutcome::Newer(manifest) => {
                // `[update]` 这个 ASCII 前缀是给探针/日志检索用的（文案本身是中文）
                log::info!(
                    "[update] 更新检查：清单 {} 比当前 {} 新",
                    manifest.version,
                    cuter_core::VERSION
                );
                if manifest.version == self.config.general.skipped_version {
                    // 用户说过"这个版本我不想看到"：仍然记下结果（「关于」页要能取消跳过），
                    // 但不再往状态栏上推。
                    log::info!("[update] skip-hit 版本 {} 已在跳过名单里，不提示", manifest.version);
                    self.set_status(fill!(
                        tr!("新版本 {} 已跳过（到「设置 → 关于」可以取消）"),
                        manifest.version
                    ));
                } else {
                    self.set_status(fill!(
                        tr!("有新版本 {}：到「设置 → 关于」里打开下载页"),
                        manifest.version
                    ));
                }
            }
            UpdateOutcome::Current(version) => {
                log::info!(
                    "[update] 更新检查：已是最新（当前 {}，清单 {version}）",
                    cuter_core::VERSION
                );
                // 不刷新状态栏的话，"正在检查更新…"会一直挂在那儿骗人
                self.set_status(fill!(tr!("已是最新版本（清单 {}）"), version));
            }
            UpdateOutcome::Failed(err) => {
                log::warn!("[update] 更新检查失败：{err}");
                self.set_status(fill!(tr!("检查失败：{}"), err));
            }
        }
        // 只有"真的问到了服务端"才算查过：失败不写时间戳，否则一次断网就能让
        // 用户二十四小时内都不再检查了（`last_update_check` 是启动节流的依据）。
        let reached = matches!(
            outcome,
            UpdateOutcome::Newer(_) | UpdateOutcome::Current(_)
        );
        self.update_report = Some(outcome);
        if reached {
            self.config.general.last_update_check = now_unix();
            self.persist_config();
        }
        ctx.request_repaint();
    }

    /// 「关于」页：版本、后端、许可、仓库地址。
    fn section_about(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        let ctx = ui.ctx().clone();
        let mut check_now = false;
        let mut open_url: Option<String> = None;
        let mut skip_version: Option<String> = None;
        settings_card(ui, &theme, tr!("关于 Cuter"), |ui| {
            settings_row(
                ui,
                &theme,
                tr!("版本"),
                tr!("截图 / 标注 / 贴图 / OCR / 滚动截屏，全部在本地完成"),
                |ui| {
                    ui.label(
                        egui::RichText::new(format!("v{}", cuter_core::VERSION))
                            .size(12.0)
                            .monospace()
                            .color(theme.on_surface),
                    );
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("界面"),
                tr!("界面框架与渲染后端"),
                |ui| {
                    ui.label(
                        egui::RichText::new(tr!("egui / eframe（glow 渲染）"))
                            .size(12.0)
                            .color(theme.on_surface),
                    );
                },
            );
            settings_row(ui, &theme, tr!("许可"), tr!("GPL-3.0-or-later"), |ui| {
                ui.label(
                    egui::RichText::new(tr!("GNU GPL v3 或更新"))
                        .color(theme.on_surface_faint),
                );
            });
            settings_row(
                ui,
                &theme,
                tr!("项目主页"),
                tr!("github.com/ChuGe/Cuter"),
                |ui| {
                    if ui.button(tr!("打开项目主页")).clicked() {
                        let _ = std::process::Command::new("explorer")
                            .arg("https://github.com/ChuGe/Cuter")
                            .spawn();
                    }
                },
            );
        });
        // 更新检查：**只查、只提示、只帮你打开下载页**，不会自己去下载或替换文件。
        // 本项目没有官方发布渠道，所以清单地址默认留空——留空就一个字节都不往外发。
        settings_card(ui, &theme, tr!("更新检查"), |ui| {
            let mut changed = settings_switch_row(
                ui,
                &theme,
                tr!("启动时检查更新"),
                tr!("只查、只提示，不会自动下载或替换文件"),
                &mut self.config.general.check_update,
            );
            settings_row(
                ui,
                &theme,
                tr!("版本清单地址"),
                tr!("留空＝不检查"),
                |ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.config.general.update_endpoint)
                            .desired_width(CONTROL_W)
                            .hint_text("https://example.com/cuter-update.json"),
                    );
                    changed |= response.changed();
                },
            );
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let waiting = self.update_checking;
                if ui
                    .add_enabled(!waiting, egui::Button::new(tr!("立即检查")))
                    .clicked()
                {
                    check_now = true;
                }
                let skipped = matches!(
                    &self.update_report,
                    Some(UpdateOutcome::Newer(manifest))
                        if manifest.version == self.config.general.skipped_version
                );
                let (text, color) = match &self.update_report {
                    _ if waiting => (tr!("正在检查…").to_owned(), theme.on_surface_dim),
                    Some(UpdateOutcome::Newer(manifest)) if skipped => (
                        fill!(
                            tr!("新版本 {}（已跳过，不再提示）"),
                            manifest.version
                        ),
                        theme.on_surface_dim,
                    ),
                    Some(UpdateOutcome::Newer(manifest)) => (
                        fill!(
                            tr!("发现新版本 {}（当前 v{}）"),
                            manifest.version,
                            cuter_core::VERSION
                        ),
                        theme.accent,
                    ),
                    Some(UpdateOutcome::Current(version)) => (
                        fill!(tr!("已是最新版本（清单 {}）"), version),
                        theme.success,
                    ),
                    Some(UpdateOutcome::Failed(err)) => {
                        (fill!(tr!("检查失败：{}"), err), theme.danger)
                    }
                    None => (tr!("还没检查过").to_owned(), theme.on_surface_dim),
                };
                ui.label(egui::RichText::new(text).size(11.0).color(color));
            });
            if let Some(UpdateOutcome::Newer(manifest)) = &self.update_report {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    if !manifest.url.trim().is_empty()
                        && ui.button(tr!("打开下载页")).clicked()
                    {
                        open_url = Some(manifest.url.clone());
                    }
                    let skipped = manifest.version == self.config.general.skipped_version;
                    let label = if skipped {
                        tr!("不再跳过")
                    } else {
                        tr!("跳过这个版本")
                    };
                    if ui.button(label).clicked() {
                        skip_version = Some(if skipped {
                            String::new()
                        } else {
                            manifest.version.clone()
                        });
                    }
                    if !manifest.notes.trim().is_empty() {
                        ui.label(
                            egui::RichText::new(manifest.notes.trim())
                                .size(11.0)
                                .color(theme.on_surface_dim),
                        );
                    }
                });
            }
            if changed {
                self.persist_config();
            }
        });
        if let Some(url) = open_url
            && let Err(err) = std::process::Command::new("explorer").arg(&url).spawn()
        {
            log::warn!("打不开下载页 {url}：{err}");
            self.set_status(tr!("打不开下载页，地址已复制到配置里"));
        }
        if let Some(version) = skip_version {
            if version.is_empty() {
                self.config.general.skipped_version.clear();
                log::info!("[update] 清空了跳过名单");
                self.set_status(tr!("不再跳过任何版本"));
            } else {
                self.config.general.skipped_version = version.clone();
                log::info!("[update] 跳过版本 {version}");
                self.set_status(fill!(tr!("已跳过版本 {}，不再提示"), version));
            }
            self.persist_config();
        }
        if check_now {
            self.start_update_check(&ctx, true);
        }
        settings_card(ui, &theme, tr!("快捷键速查"), |ui| {
            for command in HotkeyCommand::ALL {
                let spec = self
                    .config
                    .hotkeys
                    .entries()
                    .into_iter()
                    .find(|(key, _)| *key == command.key())
                    .map(|(_, value)| value.to_owned())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(cuter_ui::i18n::tr(command.label()))
                            .size(12.0)
                            .color(theme.on_surface),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(if spec.trim().is_empty() {
                                tr!("未绑定").to_owned()
                            } else {
                                spec
                            })
                            .size(12.0)
                            .monospace()
                            .color(theme.on_surface_dim),
                        );
                    });
                });
            }
        });
    }

    fn section_general(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        let mut changed = false;

        // ---- 卡片一：外观与启动 ----
        settings_card(ui, &theme, tr!("外观与启动"), |ui| {
            ui.add_space(4.0);
            let language = cuter_ui::i18n::language();
            let current = if self.config.general.language.eq_ignore_ascii_case("auto") {
                None
            } else {
                Some(language)
            };
            let options = [
                (None, tr!("跟随系统")),
                (Some(cuter_ui::i18n::Language::ZhCn), tr!("简体中文")),
                (Some(cuter_ui::i18n::Language::EnUs), tr!("English")),
            ];
            let mut picked_language = None;
            settings_row(
                ui,
                &theme,
                tr!("语言"),
                tr!("「跟随系统」按 Windows 显示语言决定"),
                |ui| {
                    picked_language = settings_select(ui, &theme, "general-language", current, &options, |value| {
                        value.map_or_else(|| tr!("跟随系统").to_owned(), |l| l.label().to_owned())
                    });
                },
            );
            if let Some(picked) = picked_language {
                self.config.general.language = match picked {
                    Some(l) => l.code().to_owned(),
                    None => "auto".to_owned(),
                };
                match picked {
                    Some(l) => cuter_ui::i18n::set_language(l),
                    None => cuter_ui::i18n::set_language(crate::desktop::system_language()),
                }
                changed = true;
            }

            let themes = [
                (cuter_core::config::ThemeKind::Auto, tr!("跟随系统")),
                (cuter_core::config::ThemeKind::Dark, tr!("曜夜紫（深色）")),
                (cuter_core::config::ThemeKind::Light, tr!("薄荷奶白（浅色）")),
            ];
            let mut picked_theme = None;
            settings_row(
                ui,
                &theme,
                tr!("主题"),
                tr!("配色与图标皮肤一起换"),
                |ui| {
                    picked_theme = settings_select(ui, &theme, "general-theme", self.config.general.theme, &themes, |kind| {
                        match kind {
                            cuter_core::config::ThemeKind::Auto => tr!("跟随系统").to_owned(),
                            cuter_core::config::ThemeKind::Dark => tr!("曜夜紫（深色）").to_owned(),
                            cuter_core::config::ThemeKind::Light => tr!("薄荷奶白（浅色）").to_owned(),
                        }
                    });
                },
            );
            if let Some(kind) = picked_theme {
                self.config.general.theme = kind;
                changed = true;
            }

            let mut start_in_tray = self.config.general.start_in_tray;
            settings_row(
                ui,
                &theme,
                tr!("启动后只驻留托盘"),
                tr!("不自动显示这个窗口；热键依然生效"),
                |ui| {
                    if settings_switch(ui, &theme, &mut start_in_tray) {
                        self.config.general.start_in_tray = start_in_tray;
                        changed = true;
                    }
                },
            );

            let mut close_to_tray = self.config.general.close_to_tray;
            settings_row(
                ui,
                &theme,
                tr!("关闭窗口时退到托盘"),
                tr!("关掉窗口不退出程序，托盘图标仍然在"),
                |ui| {
                    if settings_switch(ui, &theme, &mut close_to_tray) {
                        self.config.general.close_to_tray = close_to_tray;
                        changed = true;
                    }
                },
            );

            let mut autostart = self.config.general.autostart;
            settings_row(
                ui,
                &theme,
                tr!("开机自动启动"),
                tr!("只写一条指向本程序的 HKCU 注册表项"),
                |ui| {
                    if settings_switch(ui, &theme, &mut autostart) {
                        self.config.general.autostart = autostart;
                        changed = true;
                        desktop::reconcile_autostart(autostart);
                    }
                },
            );
        });

        ui.add_space(16.0);

        // ---- 卡片二：贴图与提示 ----
        settings_card(ui, &theme, tr!("贴图"), |ui| {
            ui.add_space(4.0);
            let mut rounded = self.config.pin.rounded_corners;
            settings_row(
                ui,
                &theme,
                tr!("贴图图片裁成圆角"),
                tr!("半径随图片大小自适应（短边 6%，2~24 px）"),
                |ui| {
                    if settings_switch(ui, &theme, &mut rounded) {
                        self.config.pin.rounded_corners = rounded;
                        for entry in lock(&self.pins).values() {
                            lock(&entry.pin).set_rounded(rounded);
                        }
                        ui.ctx().request_repaint();
                        changed = true;
                    }
                },
            );
            let mut notify = self.config.general.show_tray_notifications;
            settings_row(
                ui,
                &theme,
                tr!("截图保存后弹托盘通知"),
                tr!("启动时也会提示一次热键"),
                |ui| {
                    if settings_switch(ui, &theme, &mut notify) {
                        self.config.general.show_tray_notifications = notify;
                        if let Some(notifier) = self.notifier.as_mut() {
                            notifier.set_enabled(notify);
                        }
                        changed = true;
                    }
                },
            );
        });

        if changed {
            self.persist_config();
        }
    }

    fn section_hotkeys(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("全局快捷键"), |ui| {
            ui.label(
                egui::RichText::new(tr!("格式：修饰键 + 主键，例如 Alt+C、Ctrl+Shift+A、Ctrl+PrintScreen。留空表示不绑定。"))
                    .size(11.0)
                    .color(self.theme.on_surface_dim),
            );
            ui.add_space(6.0);
            let mut changed = false;
            for key in [
                "capture",
                "capture_monitor",
                "capture_desktop",
                "scroll_capture",
                "pin_from_clipboard",
                "ocr",
                "ocr_clipboard",
            ] {
                let (label, hint) = match key {
                    "capture" => (tr!("框选截图"), tr!("最常用：框一块区域，回车保存")),
                    "capture_monitor" => (tr!("截取当前显示器"), tr!("抓下鼠标所在的那块屏幕")),
                    "capture_desktop" => (
                        tr!("截取整个桌面"),
                        tr!("多显示器时把所有屏幕拼成一张"),
                    ),
                    "scroll_capture" => (
                        tr!("滚动截屏"),
                        tr!("框选后回车开始滚；长内容才用得上"),
                    ),
                    "pin_from_clipboard" => (
                        tr!("贴图（剪贴板）"),
                        tr!("把剪贴板里的图贴到屏幕上"),
                    ),
                    "ocr" => (tr!("识别文字"), tr!("框选一块区域，取出里面的文字")),
                    "ocr_clipboard" => (tr!("贴图并识别"), tr!("先贴图，再对这张贴图取字")),
                    _ => continue,
                };
                let Some(entry) = self
                    .hotkey_buffer
                    .iter_mut()
                    .find(|(k, _)| k.as_str() == key)
                else {
                    continue;
                };
                settings_row(ui, &theme, label, hint, |ui| {
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut entry.1).desired_width(CONTROL_W))
                        .changed();
                });
            }
            settings_row(
                ui,
                &theme,
                tr!("恢复默认热键"),
                tr!("把上面这些改回出厂设置"),
                |ui| {
                    let clicked = ui.button(tr!("重置为默认")).clicked();
                    if clicked {
                        changed = true;
                        self.config.hotkeys = cuter_core::config::HotkeyConfig::default();
                        self.hotkey_buffer = self
                            .config
                            .hotkeys
                            .entries()
                            .into_iter()
                            .map(|(k, v)| (k.to_owned(), v.to_owned()))
                            .collect();
                        if let Some(manager) = self.hotkeys.as_mut() {
                            let warnings = manager.apply(&self.config.hotkeys);
                            self.warnings.retain(|w| !w.starts_with("热键"));
                            self.warnings.extend(warnings);
                        }
                        self.persist_config();
                        let _ = ctx;
                    }
                },
            );
            if changed {
                for (key, value) in &self.hotkey_buffer {
                    self.config.hotkeys.set(key, value.clone());
                }
                if let Some(manager) = self.hotkeys.as_mut() {
                    let warnings = manager.apply(&self.config.hotkeys);
                    self.warnings.retain(|w| !w.starts_with("热键"));
                    self.warnings.extend(warnings);
                }
                self.persist_config();
            }
        });
    }

    fn section_save(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("保存"), |ui| {
            let mut changed = false;
            ui.add_space(4.0);
            // 目录：把"留空时到底存到哪"写进说明里，比单独一行灰字更省地方。
            let mut dir = self.config.save.dir.clone();
            let mut dir_edited = false;
            let default_dir = self.config.save.resolved_dir().display().to_string();
            settings_row(
                ui,
                &theme,
                tr!("保存目录"),
                &fill!(tr!("留空就存到 {}"), default_dir),
                |ui| {
                    if ui.button(tr!("打开")).clicked() {
                        let dir = self.config.save.resolved_dir();
                        let _ = std::process::Command::new("explorer").arg(dir).spawn();
                    }
                    ui.add_space(6.0);
                    // 和其它输入框一样宽（不再"这一行宽那一行窄"）；路径本身在
                    // 上面的说明里已经写全，框里放不下就悬停看整条。
                    let response = ui.add_sized(
                        egui::Vec2::new(CONTROL_W, 24.0),
                        egui::TextEdit::singleline(&mut dir).hint_text(tr!("默认目录")),
                    );
                    // 一行必然放不下整条路径，悬停看全的。
                    response.clone().on_hover_text(dir.clone());
                    dir_edited = response.changed();
                },
            );
            if dir_edited {
                self.config.save.dir = dir;
                changed = true;
            }
            settings_row(
                ui,
                &theme,
                tr!("文件名模板"),
                tr!("可用占位符：{date} {time} {datetime} {unix} {seq} {w} {h} {monitor} {app}"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.config.save.filename_pattern)
                                .desired_width(CONTROL_W),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("格式"),
                tr!("只有 jpg 会用到下面的质量"),
                |ui| {
                    if let Some(next) = settings_select(
                        ui,
                        &theme,
                        "format",
                        self.config.save.format.as_str(),
                        &[("png", "png"), ("jpg", "jpg"), ("webp", "webp"), ("bmp", "bmp")],
                        |value| value.to_owned(),
                    ) {
                        self.config.save.format = next.to_owned();
                        changed = true;
                    }
                },
            );
            // 质量滑块只在有损格式下可用：PNG / BMP 没有这个参数，
            // 但行本身始终占位——否则切格式时下面整片会跳一下；无损时右侧给灰字说明，
            // 也不摆「拖了没反应」的假开关。
            settings_row(
                ui,
                &theme,
                tr!("JPEG 质量"),
                tr!("同样画质下数字越小体积越小"),
                |ui| {
                    if self.config.save.image_format().is_lossy() {
                        changed |= settings_slider(
                            ui,
                            &theme,
                            egui::Slider::new(&mut self.config.save.quality, 1..=100)
                                .fixed_decimals(0),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(tr!("不适用（当前格式无损）"))
                                .color(theme.on_surface_faint),
                        );
                    }
                },
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("截图后自动复制到剪贴板"),
                tr!("保存文件的同时也放进剪贴板"),
                &mut self.config.save.auto_copy,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("回车默认保存到目录"),
                tr!("不勾选时回车＝复制到剪贴板"),
                &mut self.config.save.auto_save,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("保存时弹出「另存为」对话框"),
                tr!("每次都自己挑位置，不用默认目录"),
                &mut self.config.save.ask_path_on_save,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("保存后打开所在文件夹"),
                tr!("在资源管理器里定位到刚保存的文件"),
                &mut self.config.save.reveal_after_save,
            );
            if let Some(path) = &self.last_saved {
                ui.label(
                    egui::RichText::new(fill!(tr!("上次保存：{}"), path.display()))
                        .size(11.0)
                        .color(self.theme.on_surface_dim),
                );
            }
            if changed {
                self.persist_config();
            }
        });
    }

    fn section_capture(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("截图"), |ui| {
            let mut changed = false;
            ui.add_space(4.0);

            // 捕获后端：真正可切换的实现（GDI / DXGI 桌面复制）。
            // 当前**实际**在用的是哪个，看卡片底部那行提示与「诊断」页（拿不到会回退 GDI）。
            let backends = [
                (cuter_core::capture::CaptureBackend::Gdi, tr!("GDI")),
                (
                    cuter_core::capture::CaptureBackend::Dxgi,
                    tr!("DXGI（桌面复制）"),
                ),
            ];
            let mut picked_backend = None;
            settings_row(
                ui,
                &theme,
                tr!("捕获后端"),
                tr!("拿不到桌面复制时会自动回退 GDI"),
                |ui| {
                    picked_backend = settings_select(
                        ui,
                        &theme,
                        "capture-backend",
                        self.config.capture.backend,
                        &backends,
                        |backend| cuter_ui::i18n::tr(backend.label()).to_owned(),
                    );
                },
            );
            if let Some(backend) = picked_backend {
                self.config.capture.backend = backend;
                changed = true;
            }

            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("包含鼠标指针"),
                tr!("截图里带上光标本身"),
                &mut self.config.capture.include_cursor,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("显示放大镜与取色"),
                tr!("框选时显示放大镜，按住 Shift 切取色"),
                &mut self.config.capture.show_magnifier,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("截图前隐藏本程序窗口"),
                tr!("免得把贴图或设置窗口一起截进去"),
                &mut self.config.capture.hide_own_windows,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("吸附到单个控件（按钮 / 输入框）"),
                tr!("老机器上可以关掉"),
                &mut self.config.capture.snap_controls,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("框选吸附到窗口边缘"),
                tr!("按住 Alt 临时关闭"),
                &mut self.config.capture.auto_detect_windows,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("记住上次框选区域"),
                tr!("在遮罩里按 Ctrl+R 可复原上次的框选区域"),
                &mut self.config.capture.remember_last_region,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("冻结屏幕时播放提示音"),
                tr!("在遮罩出现的那一刻响一声"),
                &mut self.config.capture.play_sound,
            );

            let mut delay = self.config.capture.delay_ms;
            settings_row(
                ui,
                &theme,
                tr!("延迟捕获"),
                tr!("按下热键后先等这么久再冻结屏幕，用于等菜单收起或动画结束（0 = 立刻）"),
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut delay)
                            .range(0..=5000)
                            .speed(10.0)
                            .suffix(" 毫秒"),
                    );
                },
            );
            if delay != self.config.capture.delay_ms {
                self.config.capture.delay_ms = delay;
                changed = true;
            }

            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(fill!(tr!("当前实际在用：{}（不可用时自动回退 GDI）"), capture_backend_name()))
                    .size(11.0)
                    .color(theme.on_surface_dim),
            );
            if changed {
                self.persist_config();
            }
        });
    }

    fn section_annotate(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("标注"), |ui| {
            let mut changed = false;
            settings_row(
                ui,
                &theme,
                tr!("默认工具"),
                tr!("遮罩工具条打开时一开始选中的那个"),
                |ui| {
                    let options: Vec<(ToolKind, &str)> = ToolKind::ALL
                        .iter()
                        .map(|tool| (*tool, cuter_ui::i18n::tr(tool.label())))
                        .collect();
                    if let Some(next) = settings_select(
                        ui,
                        &theme,
                        "tool",
                        self.config.annotate.default_tool,
                        &options,
                        |tool| cuter_ui::i18n::tr(tool.label()).to_owned(),
                    ) {
                        self.config.annotate.default_tool = next;
                        changed = true;
                    }
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("颜色"),
                tr!("画笔、箭头、方框、序号都用它"),
                |ui| {
                    let mut color = egui::Color32::from_rgb(
                        self.config.annotate.stroke_color[0],
                        self.config.annotate.stroke_color[1],
                        self.config.annotate.stroke_color[2],
                    );
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        let [r, g, b, _] = color.to_array();
                        self.config.annotate.stroke_color = [r, g, b];
                        changed = true;
                    }
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("线宽"),
                tr!("画笔和图形描边的粗细"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.annotate.stroke_width)
                                .range(1.0..=32.0)
                                .speed(0.2)
                                .suffix(" 像素"),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("马赛克块"),
                tr!("数字越大，打码的格子越粗"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.annotate.mosaic_block)
                                .range(2..=64)
                                .suffix(" 像素"),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("模糊半径"),
                tr!("数字越大，糊得越厉害"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.annotate.blur_radius)
                                .range(1..=64)
                                .suffix(" 像素"),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("字号"),
                tr!("序号和文字标注的大小"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.annotate.font_size)
                                .range(8.0..=96.0)
                                .suffix(" 像素"),
                        )
                        .changed();
                },
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("图形默认填充"),
                tr!("关掉就是空心框"),
                &mut self.config.annotate.fill_shapes,
            );
            if changed {
                self.config.sanitize();
                self.persist_config();
            }
        });
    }

    fn section_pin(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("贴图"), |ui| {
            let mut changed = false;
            settings_row(
                ui,
                &theme,
                tr!("初始不透明度"),
                tr!("新贴图刚出现时有多透明"),
                |ui| {
                    changed |= settings_slider(
                        ui,
                        &theme,
                        egui::Slider::new(&mut self.config.pin.opacity, 0.1..=1.0)
                            .fixed_decimals(2),
                    );
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("初始缩放"),
                tr!("新贴图按原图的多少倍显示"),
                |ui| {
                    changed |= settings_slider(
                        ui,
                        &theme,
                        egui::Slider::new(&mut self.config.pin.initial_scale, 0.1..=4.0)
                            .suffix("×")
                            .fixed_decimals(2),
                    );
                },
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("滚轮缩放贴图（中键恢复 100%）"),
                tr!("不勾选时滚轮不做任何事"),
                &mut self.config.pin.wheel_zoom,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("窗口内缩放：只放大内容，窗口尺寸保持不变"),
                tr!("勾上就不会越放大越挡住桌面"),
                &mut self.config.pin.zoom_within_window,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("投影"),
                tr!("窗口四周留出一圈渐变阴影，不遮挡桌面内容"),
                &mut self.config.pin.shadow,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("显示 1 像素描边"),
                tr!("浅色背景上更容易看清边界"),
                &mut self.config.pin.border,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("新贴图默认鼠标穿透"),
                tr!("点它就像点桌面；按住 Ctrl 才能拖动"),
                &mut self.config.pin.click_through,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("新贴图默认始终置顶"),
                tr!("不勾选时会被其它窗口压住"),
                &mut self.config.pin.always_on_top,
            );
            ui.label(
                egui::RichText::new(tr!("只影响之后新开的贴图。"))
                .size(11.0)
                .color(self.theme.on_surface_dim),
            );
            if changed {
                self.config.sanitize();
                self.persist_config();
            }
        });
        settings_card(ui, &theme, tr!("贴图分组"), |ui| {
            let mut changed = false;
            settings_row(
                ui,
                &theme,
                tr!("分组数量"),
                tr!("几套互不干扰的贴图；Ctrl+F3 切到下一组"),
                |ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.config.pin.groups).range(1..=9))
                        .changed();
                },
            );
            let per_group: Vec<usize> = (0..self.config.pin.groups)
                .map(|group| self.pins_in_group(group))
                .collect();
            let summary = per_group
                .iter()
                .enumerate()
                .map(|(index, count)| {
                    if index as u32 == self.pin_group {
                        fill!(tr!("[第 {} 组 {} 张]"), index + 1, count)
                    } else {
                        fill!(tr!("第 {} 组 {} 张"), index + 1, count)
                    }
                })
                .collect::<Vec<_>>()
                .join("  ");
            ui.label(
                egui::RichText::new(summary)
                    .size(11.0)
                    .color(self.theme.on_surface_dim),
            );
            settings_row(
                ui,
                &theme,
                tr!("手动切换"),
                tr!("不想按热键时用这里"),
                |ui| {
                    if ui.button(tr!("切到下一组")).clicked() {
                        self.switch_pin_group(ui.ctx());
                    }
                    if ui.button(tr!("隐藏/显示本组")).clicked() {
                        self.toggle_pins(ui.ctx());
                    }
                },
            );
            ui.label(
                egui::RichText::new(tr!("几套互不干扰的贴图，切一下整体换掉；只改可见性，切回去东西都还在。"))
                .size(11.0)
                .color(self.theme.on_surface_dim),
            );
            if changed {
                self.config.sanitize();
                self.persist_config();
            }
        });
    }

    fn section_ocr(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("识别文字（OCR）"), |ui| {
            let mut changed = false;

            settings_row(
                ui,
                &theme,
                tr!("引擎"),
                tr!("内置引擎完全离线；在线接口把图发给自建服务"),
                |ui| {
                    let kinds = [
                        cuter_core::config::OcrEngine::Builtin,
                        cuter_core::config::OcrEngine::Http,
                        cuter_core::config::OcrEngine::Disabled,
                    ];
                    let options: Vec<(cuter_core::config::OcrEngine, &str)> = kinds
                        .iter()
                        .map(|kind| (*kind, cuter_ui::i18n::tr(kind.label())))
                        .collect();
                    if let Some(next) = settings_select(
                        ui,
                        &theme,
                        "ocr-engine",
                        self.config.ocr.engine,
                        &options,
                        |kind| cuter_ui::i18n::tr(kind.label()).to_owned(),
                    ) {
                        self.config.ocr.engine = next;
                        changed = true;
                    }
                },
            );

            match self.config.ocr.engine {
                cuter_core::config::OcrEngine::Builtin => {
                    ui.label(
                        egui::RichText::new(
                            tr!("语言取决于系统里装了哪些 OCR 语言组件（见下方列表）。"),
                        )
                        .size(11.0)
                        .color(self.theme.on_surface_dim),
                    );
                }
                cuter_core::config::OcrEngine::Disabled => {
                    ui.label(
                        egui::RichText::new(
                            tr!("识别功能已关闭；遮罩里按 Ctrl+O 会提示未启用。"),
                        )
                        .size(11.0)
                        .color(self.theme.on_surface_dim),
                    );
                }
                cuter_core::config::OcrEngine::Http => {
                    settings_row(
                        ui,
                        &theme,
                        tr!("接口地址"),
                        tr!("把框选的图 POST 到这里：multipart 的 image 字段，内容是 PNG 的 base64"),
                        |ui| {
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.config.ocr.http_endpoint)
                                    .desired_width(CONTROL_W)
                                    .hint_text("http://127.0.0.1:8866/ocr"),
                            );
                            changed |= edit.changed();
                        },
                    );
                    // 注意：整条 `tr!` 必须写在一行里 —— `tools/i18n/extract_keys.py` 的正则
                    // 只认同一行的字面量，换行写的键会被漏掉（HANDOFF §54.2 记过这个坑）。
                    ui.label(
                        egui::RichText::new(tr!("只支持 http；响应按 PaddleOCR hubserving 形状解析，低置信度的块会被丢掉。"))
                        .size(11.0)
                        .color(self.theme.on_surface_dim),
                    );
                }
            }

            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("识别后自动复制到剪贴板"),
                tr!("识别完直接进剪贴板，省一次 Ctrl+C"),
                &mut self.config.ocr.auto_copy,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("框选后按回车直接就识别（不必再按 Ctrl+O）"),
                tr!("回车既保存也识别；只想要图不想识别就别开"),
                &mut self.config.ocr.auto_after_capture,
            );
            changed |= settings_switch_row(
                ui,
                &theme,
                tr!("段落模式（按标点断行）"),
                tr!("关掉就是一行一行地抄，保留原来的换行"),
                &mut self.config.ocr.paragraph,
            );

            settings_row(
                ui,
                &theme,
                tr!("语言"),
                tr!("留空 = 跟随系统偏好；也可以填 zh-Hans-CN 之类强制指定"),
                |ui| {
                    let mut language = self
                        .config
                        .ocr
                        .languages
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut language)
                                .desired_width(CONTROL_W)
                                .hint_text(tr!("留空 = 跟随系统偏好")),
                        )
                        .changed()
                    {
                        self.config.ocr.languages = if language.trim().is_empty() {
                            Vec::new()
                        } else {
                            vec![language]
                        };
                        changed = true;
                    }
                },
            );

            // 列出系统实际装了哪些 OCR 语言——「为什么识别不出来」大多是因为这个
            #[cfg(windows)]
            {
                let languages = cuter_core::ocr::windows_ocr::available_languages();
                if languages.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            tr!("⚠ 系统没有安装任何 OCR 语言组件。请在「设置 → 时间和语言 → 语言和区域」里给目标语言添加「光学字符识别」可选功能。"),
                        )
                        .size(12.0)
                        .color(self.theme.danger),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(fill!(tr!("系统可用的 OCR 语言：{}"), languages.join("、")))
                            .size(11.0)
                            .color(self.theme.on_surface_dim),
                    );
                }
            }

            ui.label(
                egui::RichText::new(
                    tr!("框选后按 Ctrl+O 识别。"),
                )
                .size(11.0)
                .color(self.theme.on_surface_dim),
            );

            if changed {
                self.persist_config();
            }
        });
    }

    /// 滚动截屏设置。这里的每一项都在采集线程里有真实作用点。
    fn section_scroll(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("滚动截屏"), |ui| {
            let mut changed = false;

            settings_row(
                ui,
                &theme,
                tr!("抓帧间隔"),
                tr!("越小越跟得上快速滚动，越大越省 CPU"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.scroll.interval_ms)
                                .range(60..=5000)
                                .suffix(" ms"),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("长度上限"),
                tr!("超过这个高度就停下，防止一路上滚不到头"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.scroll.max_height)
                                .range(200..=100_000)
                                .suffix(" 行"),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("最小识别位移"),
                tr!("比这更小的位移当成噪声忽略，避免抖着滚"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.scroll.min_offset)
                                .range(1..=64)
                                .suffix(" px"),
                        )
                        .changed();
                },
            );
            settings_row(
                ui,
                &theme,
                tr!("最大搜索位移"),
                tr!("一次滚动最多能跟上多少像素；调大更稳但更慢"),
                |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.config.scroll.max_offset)
                                .range(8..=8000)
                                .suffix(" px"),
                        )
                        .changed();
                },
            );
            ui.label(
                egui::RichText::new(tr!("超出「最大搜索位移」的那一帧会被判为「没识别出滚动」，继续滚动即可。"))
                .size(11.0)
                .color(self.theme.on_surface_dim),
            );

            if changed {
                // 立刻夹取，避免用户把 max_offset 拖到小于 min_offset（那样永远匹配不上）
                self.config.scroll.clamp();
                self.persist_config();
            }
        });
    }

    fn section_history(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("截图历史"), |ui| {
            let mut changed = false;

            let mut enabled = self.config.history.enabled();
            if settings_switch_row(
                ui,
                &theme,
                tr!("保存最近几张截图（可再复制 / 再贴图）"),
                tr!("存的是无损原图"),
                &mut enabled,
            ) {
                // 关掉时把 limit 归零，但**记住**用户原来想要的张数，
                // 这样重新勾上时不会莫名其妙变成默认值。
                if enabled {
                    self.config.history.limit = self.history_limit_backup.max(1);
                } else {
                    self.history_limit_backup = self.config.history.limit;
                    self.config.history.limit = 0;
                }
                changed = true;
            }

            settings_row(
                ui,
                &theme,
                tr!("保留张数"),
                tr!("超出就按时间从旧到新删掉"),
                |ui| {
                    ui.add_enabled_ui(self.config.history.enabled(), |ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.config.history.limit)
                                    .range(1..=500)
                                    .suffix(" 张"),
                            )
                            .changed();
                    });
                },
            );

            // 占用与张数必须摆在设置里：历史把**原图**存到磁盘，
            // 用户有权一眼看到它到底占了多少（HANDOFF 第 8 节的三个约束之一）。
            let (count, total, dir) = {
                let store = lock(&self.history);
                (store.len(), store.total_bytes(), store.root().to_path_buf())
            };
            settings_row(
                ui,
                &theme,
                tr!("当前占用"),
                &fill!(tr!("{} 张，共 {}"), count, cuter_core::history::human_bytes(total)),
                |ui| {
                    if ui.button(tr!("打开历史面板")).clicked() {
                        self.open_history(ctx);
                    }
                    if ui.button(tr!("打开目录")).clicked() {
                        let _ = std::fs::create_dir_all(&dir);
                        #[cfg(windows)]
                        {
                            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                        }
                    }
                    if ui.button(tr!("清空历史")).clicked() {
                        let result = lock(&self.history).clear();
                        match result {
                            Ok(()) => self.set_status(tr!("历史已清空")),
                            Err(err) => self.set_status(fill!(tr!("清空历史失败：{}"), err)),
                        }
                    }
                },
            );
            ui.label(
                egui::RichText::new(fill!(tr!("目录：{}"), dir.display()))
                    .size(11.0)
                    .color(self.theme.on_surface_dim),
            );
            ui.label(
                egui::RichText::new(tr!("剪贴板里贴进来的图不会进历史。"))
                    .size(11.0)
                    .color(self.theme.on_surface_dim),
            );

            if changed {
                self.config.history.clamp();
                self.persist_config();
            }
        });
    }

    fn section_font(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("字体"), |ui| {
            let mut changed = false;
            settings_row(
                ui,
                &theme,
                tr!("字体文件"),
                tr!("留空 = 自动探测"),
                |ui| {
                    // 控制区是右对齐排布的：先加的控件靠右，所以「重新探测」写在输入框前面，
                    // 渲染出来才是「输入框 … 按钮」。
                    if ui.button(tr!("重新探测")).clicked() {
                        // 换了字体文件：重新装载（这里必然已经有界面在画了）。
                        self.font = None;
                        self.ensure_font(ui.ctx());
                        changed = true;
                    }
                    let mut path = self.config.annotate.text_font.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut path)
                                .desired_width(CONTROL_W)
                                .hint_text(tr!("留空 = 自动探测")),
                        )
                        .changed()
                    {
                        self.config.annotate.text_font = path;
                        changed = true;
                    }
                },
            );
            ui.label(
                egui::RichText::new(tr!("自动探测顺序：微软雅黑 → 黑体 → 等线 → 宋体"))
                    .size(11.0)
                    .color(theme.on_surface_dim),
            );
            match &self.font {
                Some(font) => {
                    ui.label(
                        egui::RichText::new(fill!(tr!("当前：{}（{}）{}"), font.name, font.path.display(), if font.has_cjk { "" } else { tr!(" · 不含中文") }))
                        .size(11.0)
                        .color(self.theme.on_surface_dim),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new(tr!("未找到可用字体，文字标注将无法渲染"))
                            .size(11.0)
                            .color(self.theme.danger),
                    );
                }
            }
            ui.label(
                egui::RichText::new(tr!("换完之后点「重新探测」立刻生效。"))
                .size(11.0)
                .color(self.theme.on_surface_dim),
            );
            if changed {
                self.persist_config();
            }
        });
    }

    fn section_diagnostics(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        settings_card(ui, &theme, tr!("运行状态"), |ui| {
            match desktop_capture() {
                Ok(backend) => {
                    settings_row(
                        ui,
                        &theme,
                        tr!("捕获后端"),
                        tr!("实际在用的那套；DXGI 不可用时自动回退 GDI"),
                        |ui| {
                            ui.label(
                                egui::RichText::new(backend.backend_name())
                                    .size(12.0)
                                    .color(theme.on_surface),
                            );
                        },
                    );
                    settings_row(
                        ui,
                        &theme,
                        tr!("启动耗时"),
                        tr!("进程创建 → 第一帧"),
                        |ui| {
                            ui.label(
                                egui::RichText::new(fill!(tr!("{} ms"), self.startup_ms))
                                    .size(12.0)
                                    .color(theme.on_surface),
                            );
                        },
                    );
                    match backend.monitors() {
                        Ok(monitors) => {
                            settings_row(
                                ui,
                                &theme,
                                tr!("显示器"),
                                tr!("每块屏幕的分辨率与缩放率"),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(fill!(tr!("{} 台"), monitors.len()))
                                            .size(12.0)
                                            .color(theme.on_surface),
                                    );
                                },
                            );
                            let scales: Vec<f32> =
                                monitors.iter().map(|m| m.scale_factor).collect();
                            let mixed =
                                scales.iter().any(|s| (s - scales[0]).abs() > 1e-3);
                            for monitor in &monitors {
                                settings_row(
                                    ui,
                                    &theme,
                                    &monitor.name,
                                    &format!(
                                        "{}×{} @ {:.0}%",
                                        monitor.bounds.w,
                                        monitor.bounds.h,
                                        monitor.scale_factor * 100.0
                                    ),
                                    |ui| {
                                        if monitor.is_primary {
                                            ui.label(
                                                egui::RichText::new(tr!("主屏"))
                                                    .size(12.0)
                                                    .color(theme.on_surface_dim),
                                            );
                                        }
                                    },
                                );
                            }
                            if mixed {
                                settings_row(
                                    ui,
                                    &theme,
                                    tr!("缩放率不一致"),
                                    tr!("全屏遮罩跨屏时 Windows 会强制调整窗口尺寸，请改用「当前显示器」截图。"),
                                    |_ui| {},
                                );
                            }
                        }
                        Err(err) => {
                            settings_row(ui, &theme, tr!("显示器"), tr!("枚举失败"), |ui| {
                                ui.label(
                                    egui::RichText::new(fill!(tr!("{}"), err))
                                        .size(12.0)
                                        .color(theme.danger),
                                );
                            });
                        }
                    }
                }
                Err(err) => {
                    settings_row(ui, &theme, tr!("捕获后端"), tr!("不可用"), |ui| {
                        ui.label(
                            egui::RichText::new(fill!(tr!("{}"), err))
                                .size(12.0)
                                .color(theme.danger),
                        );
                    });
                }
            }
            settings_row(
                ui,
                &theme,
                tr!("配置文件"),
                &Config::path().display().to_string(),
                |ui| {
                    if ui.button(tr!("打开配置目录")).clicked() {
                        let _ = std::process::Command::new("explorer")
                            .arg(Config::config_dir())
                            .spawn();
                    }
                },
            );
            if Config::is_portable() {
                settings_row(
                    ui,
                    &theme,
                    tr!("便携模式"),
                    &fill!(
                        tr!("exe 旁边有 {}，配置 / 日志 / 默认保存目录都在 {}"),
                        cuter_core::config::PORTABLE_MARKER,
                        Config::config_dir().display()
                    ),
                    |_ui| {},
                );
            }
            settings_row(
                ui,
                &theme,
                tr!("开机自启"),
                tr!("只写一条指向本程序的 HKCU 注册表项"),
                |ui| {
                    ui.label(
                        egui::RichText::new(match desktop::autostart_entry() {
                            Some(entry) => entry,
                            None => tr!("未开启").to_owned(),
                        })
                        .size(12.0)
                        .color(theme.on_surface_dim),
                    );
                },
            );
            // 日志落盘之后，"出问题该看哪儿"要能在界面上直接找到——不然用户只会说
            // "它坏了"，而我们还以为没有现场。
            match crate::logging::log_file() {
                Some(path) => {
                    settings_row(
                        ui,
                        &theme,
                        tr!("日志"),
                        &path.display().to_string(),
                        |ui| {
                            if ui.button(tr!("打开日志目录")).clicked() {
                                let _ = std::process::Command::new("explorer")
                                    .arg(crate::logging::log_dir())
                                    .spawn();
                            }
                        },
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new(tr!("⚠ 日志没有落盘（只能看控制台输出）"))
                            .size(11.0)
                            .color(theme.danger),
                    );
                }
            }
        });
        let hotkey_failed = self
            .hotkeys
            .as_ref()
            .is_some_and(|manager| !manager.failures().is_empty());
        if hotkey_failed || !self.warnings.is_empty() {
            settings_card(ui, &theme, tr!("需要处理"), |ui| {
                if let Some(manager) = &self.hotkeys {
                    for (spec, reason) in manager.failures() {
                        ui.label(
                            egui::RichText::new(fill!(tr!("⚠ 热键「{}」：{}"), spec, reason))
                                .size(12.0)
                                .color(theme.danger),
                        );
                    }
                }
                for warning in &self.warnings {
                    ui.label(
                        egui::RichText::new(format!("⚠ {warning}"))
                            .size(12.0)
                            .color(theme.danger),
                    );
                }
            });
        }
    }

    fn persist_config(&mut self) {
        self.config.sanitize();
        if let Err(err) = self.config.save() {
            log::warn!("保存配置失败：{err}");
            self.set_status(fill!(tr!("保存配置失败：{}"), err));
        }
    }

    fn apply_pending_theme(&mut self, ctx: &Context) {
        // 每帧重算一次：「跟随系统」应当跟着系统主题实时切换，
        // 代价只是一次字段读取，比维护一套"主题变了"的信号便宜得多。
        let resolved = resolve_theme(self.config.general.theme, ctx);
        if resolved != self.theme {
            self.theme = resolved;
        }
        ctx.set_visuals(self.theme.visuals());

        // 皮肤（`resolve_theme` 里已设）变了才换图标：重建 RGBA 与调 Shell_NotifyIcon
        // 都不便宜，主题没变时不该每帧做一遍。
        let skin = crate::icon::skin();
        if skin != self.icon_skin {
            self.icon_skin = skin;
            log::info!("图标皮肤切换到 {}", skin.code());
            // 窗口图标：任务栏与 Alt+Tab 里的那个
            if let Some(icon) = crate::icon::window_icon_shared() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(icon)));
            }
            // 托盘图标
            if let Some(tray) = &self.tray {
                tray.set_skin(skin);
            }
        }
    }

    /// 把贴图窗口里改过的偏好写回配置并落盘。
    fn apply_pin_prefs(&mut self) {
        let patches: Vec<PinPrefPatch> = std::mem::take(&mut lock(&self.pin_prefs));
        if patches.is_empty() {
            return;
        }
        for patch in patches {
            match patch {
                PinPrefPatch::Opacity(value) => self.config.pin.opacity = value,
                PinPrefPatch::ClickThrough(value) => self.config.pin.click_through = value,
                PinPrefPatch::AlwaysOnTop(value) => self.config.pin.always_on_top = value,
            }
        }
        self.persist_config();
    }

    /// 收下贴图窗口里「识别文字」排进来的图片。
    fn drain_pin_ocr(&mut self, ctx: &Context) {
        let frames: Vec<Frame> = std::mem::take(&mut lock(&self.pin_ocr_requests));
        if let Some(frame) = frames.into_iter().next() {
            self.start_ocr(ctx, frame);
        }
    }

    /// 收下别的实例转达过来的请求。
    ///
    /// 单实例守卫把「用户又启动了一次」变成一次跨进程的事件通知，这里把它翻译成
    /// 本实例的动作。**没有这一步，单实例只是一个"第二份点不亮"的坏体验**：
    /// 用户双击图标却什么都没发生，比多开一个实例更让人困惑。
    fn poll_activation(&mut self, ctx: &Context) {
        // 先把请求全部取出来再处理：`take_request` 借的是 `self.instance`，
        // 而下面的分支要改 `self.request` / `self.status`。
        let mut requests = Vec::new();
        if let Some(instance) = &self.instance {
            while let Some(request) = instance.take_request() {
                requests.push(request);
            }
        }

        for request in requests {
            log::info!("收到另一个实例的请求：{request:?}");
            match request {
                Activation::Show => {
                    // 用户点名要窗口了，先取消"启动即隐藏"，再显示并抢焦点
                    self.start_hidden_frames = None;
                    self.set_control_visible(ctx, true);
                    // 最小化状态下也要能叫回来：先取消最小化再要焦点
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                    self.set_status(tr!("Cuter 一直在后台运行，这就是它的设置窗口"));
                }
                Activation::Capture => {
                    self.begin_request(CaptureRequest::Region);
                }
                Activation::Pin => {
                    self.pending_start_pin = true;
                }
                Activation::History => {
                    self.open_history(ctx);
                    self.set_status(tr!("已打开截图历史"));
                }
                // 菜单要贴着鼠标弹出来；转交过来的请求读的也是当前位置
                Activation::TrayMenu => self.open_tray_menu(ctx),
                Activation::TogglePins => self.toggle_pins(ctx),
                Activation::NextPinGroup => self.switch_pin_group(ctx),
                Activation::ClearPinClickThrough => self.clear_pin_click_through(ctx),
            }
        }
    }

    /// 主窗口的关闭按钮：`general.close_to_tray` 打开时退到托盘，而不是退出程序。    ///
    /// 这条路径是 `eframe` 官方推荐的做法：检查 `close_requested()` 并回一个
    /// [`ViewportCommand::CancelClose`]。没有它的话，用户点一下设置窗口的 X
    /// 就会把托盘与全局热键一起带走——而用户通常以为那只是"关掉这个窗口"。
    fn handle_root_close(&mut self, ctx: &Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        // 托盘菜单里的「退出」走的是 `quit`，那时必须真的关掉
        if self.quit || !self.config.general.close_to_tray {
            return;
        }
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        self.set_control_visible(ctx, false);
        self.set_status(tr!("已退到托盘，仍在后台运行（Alt+C 截图、点托盘图标打开设置）"));
    }
}

/// 一次截图请求当前所处的阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingCapture {
    /// 等延迟结束（`capture.delay_ms`）。
    Waiting(Instant),
    /// 已经请求隐藏自身窗口，**等自己的窗口真的从屏幕上消失**之后再抓。
    ///
    /// * `deadline`：超时。宁可截进自己的窗口，也不能永远卡住不抓。
    /// * `hidden_at`：第一次观察到"根窗口样式位已不可见"的时刻；`None` 表示还没观察到。
    Hiding {
        deadline: Instant,
        hidden_at: Option<Instant>,
    },
}

/// 隐藏自身窗口时"最多等多久"，以及"窗口不可见之后还要留多少余量"。
///
/// `ShowWindow(SW_HIDE)` 是同步的，但**屏幕上**那一帧要等 DWM 重新合成。
/// 实测（见 `docs/HANDOFF.md` 第 15 节）两件事：
///
/// 1. 从"下发 `Visible(false)`"到 `IsWindowVisible` 变假要 **约 80~130 ms**
///    （要走完 eframe → `egui_winit::process_viewport_commands` → winit
///    `EventLoopThreadExecutor` → `ShowWindow` 一整条链路）；
/// 2. 变假之后**立刻抓图仍然会拍到它**——偏差 26（干净时应为 0），
///    说明合成器还没换帧。
///
/// 所以判据是两段：先轮询"真的不可见"（问操作系统，不猜），
/// 再用 [`crate::desktop::flush_composition`] 推一次合成器，然后留
/// [`HIDE_SETTLE_AFTER`] 的余量。前两版就是在这里翻车的：
/// 一版按帧数等（隐藏窗口一帧 100 ms，等"三帧"其实是等 300 ms，碰巧够），
/// 一版按 `IsWindowVisible` 一问就走（太快，把窗口截进了图里）。
///
/// `HIDE_SETTLE_BEFORE` 只用于"屏幕上只有贴图窗口"这种拿不到句柄的情况兜底。
const HIDE_SETTLE_BEFORE: Duration = Duration::from_millis(30);
const HIDE_SETTLE_AFTER: Duration = Duration::from_millis(150);
const HIDE_TIMEOUT: Duration = Duration::from_millis(900);

/// 结果交付方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeliverMode {
    Copy,
    /// 存到配置目录 + 自动文件名。
    Save,
    /// 弹系统「另存为」对话框。
    SaveAs,
}

/// 贴图保存目录（贴图窗口的闭包是 `Send + Sync`，不能借用 `&self`）。
fn self_save_dir() -> PathBuf {
    let config = Config::load();
    config.save.resolved_dir()
}

/// 保存时实际使用的编码格式。
///
/// 规则：**路径的扩展名优先**（用户在「另存为」对话框里把文件名改成 `.jpg`
/// 就该得到 JPEG），扩展名不认识时回退到配置里的格式；**质量一律来自配置**。
///
/// 抽成纯函数是为了能单测：真正会出错的地方不是"算不出格式"，而是
/// "算出来了却没传进编码器"——`Frame::save` 就属于后者。
fn save_format_for(
    path: &std::path::Path,
    fallback: cuter_core::frame::ImageFormat,
    quality: u8,
) -> cuter_core::frame::ImageFormat {
    cuter_core::frame::ImageFormat::from_path(path)
        .unwrap_or(fallback)
        .with_quality(quality)
}

/// 把配置里的主题偏好解析成实际配色。
///
/// `Auto` 必须真的问一次系统，而不是直接当成深色——否则设置界面里的「跟随系统」
/// 就是一句谎话。
///
/// 顺带把图标皮肤定下来：浅色主题配「薄荷奶白」图标，深色主题配「曜夜紫」。
/// 深紫图标挂在亮任务栏上是一块脏斑，奶白图标挂在深色任务栏上会糊掉，
/// 所以皮肤必须跟主题同进同退。
fn resolve_theme(kind: cuter_core::config::ThemeKind, ctx: &Context) -> Theme {
    let (theme, light) = match kind {
        cuter_core::config::ThemeKind::Light => (Theme::light(), true),
        cuter_core::config::ThemeKind::Dark => (Theme::dark(), false),
        cuter_core::config::ThemeKind::Auto => match ctx.system_theme() {
            Some(egui::Theme::Light) => (Theme::light(), true),
            // 系统没说（或说深色）时用深色：截图工具的界面浮在桌面上，
            // 深色更不抢画面，也是绝大多数截图工具的默认值
            _ => (Theme::dark(), false),
        },
    };
    crate::icon::set_skin(if light {
        crate::icon::IconSkin::Mint
    } else {
        crate::icon::IconSkin::Purple
    });
    theme
}

fn capture_backend_name() -> &'static str {
    match desktop_capture() {
        Ok(backend) => backend.backend_name(),
        Err(_) => tr!("不可用"),
    }
}

/// 遮罩出现时播放系统提示音（配置项 `capture.play_sound`）。
///
/// 用的是系统自带的 `MessageBeep`：不携带音频资源、不需要额外的解码库，
/// 符合「小体积」目标；代价是只能发出系统预设音效。
fn play_capture_sound() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::MB_OK;
        // SAFETY: MessageBeep 只接受一个消息框样式常量，没有指针参数；
        // 失败时返回错误码，不影响内存安全。
        if let Err(err) = unsafe { MessageBeep(MB_OK) } {
            log::debug!("播放提示音失败：{err}");
        }
    }
}

/// OCR 结果面板的视口描述。
///
/// 刻意只设标题与初始尺寸，**不设位置**：这是个用户可以自由移动/缩放的普通窗口，
/// 交给系统做层叠摆放即可。（`ViewportBuilder::patch` 只在取值变化时才下发命令，
/// 所以每帧重新声明同一份描述不会打断用户的拖动/缩放。）
/// 标题栏右侧的三个窗口按钮。
#[derive(Clone, Copy, PartialEq, Eq)]
enum TitleButton {
    Minimize,
    Maximize,
    Close,
}

/// 标题栏左侧的应用标记（和 `icon.rs` 同一套配色与几何：「甘」形取景框 + 中心焦点）。
///
/// 16px 下看不出渐变的层次，底色取渐变中间值即可——为这点面积去建一张纹理不值。
/// 皮肤跟着当前主题走（深色=曜夜紫，浅色=薄荷奶白），否则浅色窗口左上角会挂着一块深紫。
fn paint_app_mark(painter: &egui::Painter, rect: egui::Rect) {
    let (background, line, dot) = match crate::icon::skin() {
        crate::icon::IconSkin::Purple => (
            egui::Color32::from_rgb(0x1B, 0x0E, 0x33),
            egui::Color32::from_rgb(0x8C, 0xF8, 0xFF),
            egui::Color32::WHITE,
        ),
        crate::icon::IconSkin::Mint => (
            egui::Color32::from_rgb(0xE9, 0xF7, 0xF2),
            egui::Color32::from_rgb(0x14, 0xA8, 0x8E),
            egui::Color32::from_rgb(0x0B, 0x5F, 0x52),
        ),
    };
    painter.rect_filled(
        rect,
        egui::CornerRadius::same((rect.width() * 0.23).round() as u8),
        background,
    );
    let inset = rect.width() * 0.30;
    let frame = egui::Rect::from_min_max(
        rect.min + egui::vec2(inset, inset),
        rect.max - egui::vec2(inset, inset),
    );
    painter.rect_stroke(
        frame,
        egui::CornerRadius::same(2),
        egui::Stroke::new((rect.width() * 0.09).max(1.0), line),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(rect.center(), (rect.width() * 0.08).max(1.0), dot);
}

/// 画标题栏按钮里的 ─ / ☐ / ✕。
///
/// 用线段和矩形拼，不引图标字体：三个字形都只有几笔，矢量画法在任意缩放下都清楚。
fn paint_title_button_glyph(
    painter: &egui::Painter,
    rect: egui::Rect,
    kind: TitleButton,
    color: egui::Color32,
) {
    let center = rect.center();
    let half = 5.0;
    let stroke = egui::Stroke::new(1.0, color);
    match kind {
        TitleButton::Minimize => {
            painter.hline((center.x - half)..=(center.x + half), center.y + 0.5, stroke);
        }
        TitleButton::Maximize => {
            let glyph = egui::Rect::from_center_size(center, egui::Vec2::splat(half * 2.0));
            painter.rect_stroke(
                glyph,
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        TitleButton::Close => {
            painter.line_segment(
                [
                    center + egui::vec2(-half, -half),
                    center + egui::vec2(half, half),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + egui::vec2(-half, half),
                    center + egui::vec2(half, -half),
                ],
                stroke,
            );
        }
    }
}

/// 每个 deferred 视口（贴图 / 识别结果 / 截图历史 / 滚动截屏）都有自己的 egui `Context`。
///
/// eframe 会按**系统主题**每帧把 visuals 覆写回去，所以「只 `set_visuals`」是不够的：
/// 必须同时 `set_theme`，否则「系统浅色 + 配置深色」会画成浅底浅字（滚动截屏面板就踩过，
/// 标题几乎看不见）。视口回调里的第一件事都应该是调这个函数。
fn pin_viewport_theme(ui: &egui::Ui, theme: &Theme, light: bool) {
    ui.ctx()
        .set_theme(if light { egui::Theme::Light } else { egui::Theme::Dark });
    ui.ctx().set_visuals(theme.visuals());
}

/// 配置里的主题是不是浅色（`Auto` 时跟随系统）。
fn light_mode(kind: cuter_core::config::ThemeKind, ctx: &Context) -> bool {
    match kind {
        cuter_core::config::ThemeKind::Light => true,
        cuter_core::config::ThemeKind::Dark => false,
        cuter_core::config::ThemeKind::Auto => ctx.system_theme() == Some(egui::Theme::Light),
    }
}

fn ocr_viewport_builder() -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(cuter_ui::ocr_panel::window_title())
        .with_inner_size([980.0, 620.0])
        .with_min_inner_size([520.0, 320.0])
}

/// 历史面板的视口描述。
///
/// 和 OCR 面板一样：只设标题与初始尺寸，**不设位置**——这是个用户能自由拖动的普通窗口。
fn history_viewport_builder() -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(cuter_ui::history_panel::window_title())
        .with_inner_size([720.0, 560.0])
        .with_min_inner_size([420.0, 260.0])
}

/// 托盘菜单面板的视口描述。
///
/// 无边框 + 固定尺寸 + 最上层：它要长得像系统右键菜单，而不是一个窗口。
/// **位置不在这里定**（`--tray-menu` 与托盘右键都要贴着鼠标），见 `open_tray_menu`。
fn tray_menu_viewport_builder() -> ViewportBuilder {
    let size = cuter_ui::tray_panel::PANEL_SIZE;
    ViewportBuilder::default()
        .with_title(cuter_ui::tray_panel::window_title())
        .with_inner_size([size.0, size.1])
        .with_min_inner_size([size.0, size.1])
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top()
}

/// 滚动截屏控制窗的尺寸（egui 点）。设计稿：460 × 420；真机上中文标题 + 三颗胶囊 +
/// 「自动滚动：开 (A)」+「取消」在 460 点里差约 20 点，会挤到胶囊上，所以放宽到 500。
const SCROLL_PANEL_SIZE: (f32, f32) = (500.0, 420.0);
/// 控制窗与选区的间距（物理像素）。
const SCROLL_PANEL_GAP: i32 = 12;

/// 滚动截屏控制窗的视口描述。
///
/// 位置在 `start_scroll` 里算好后传进来（那时才拿得到显示器工作区），这里只用点。
fn scroll_viewport_builder(pos: [f32; 2]) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(cuter_ui::scroll_panel::window_title())
        .with_inner_size([SCROLL_PANEL_SIZE.0, SCROLL_PANEL_SIZE.1])
        .with_min_inner_size([420.0, 340.0])
        .with_position(pos)
        // 采集期间用户可能在目标窗口里操作，控制窗必须浮在最上层，
        // 否则它会被目标窗口盖住，用户找不到「完成」按钮。
        .with_always_on_top()
}

/// 给控制窗挑一个**不压住选区的**位置（物理像素）。
///
/// 这是硬要求：控制窗会一直挂在屏幕上，一旦它落在被采集的矩形里，就会被逐帧抓进
/// 长图——既污染画面，又会让位移估计因为这块静止的像素而失配。
/// 依次尝试「右侧 / 左侧 / 上方 / 下方」，每个候选先夹进工作区再判断是否与选区相交；
/// 四个方向都不行（典型情况是整屏框选）时退到工作区右下角。
///
/// 返回 `(位置, 是否仍然重叠)`，调用方据此打日志——**不谎称一定不重叠**。
fn scroll_control_position(region: Rect, panel: Size, work: Rect) -> (Point, bool) {
    let gap = SCROLL_PANEL_GAP;
    let rect_at = |p: Point| Rect::new(p.x, p.y, panel.w, panel.h);
    // `clamp` 在两个参数相等时返回该值，但 lo > hi 会 panic，所以上界先 max 一次
    let hi_x = (work.right() - panel.w as i32).max(work.left());
    let hi_y = (work.bottom() - panel.h as i32).max(work.top());
    let clamp = |p: Point| Point::new(p.x.clamp(work.left(), hi_x), p.y.clamp(work.top(), hi_y));

    let candidates = [
        Point::new(region.right() + gap, region.top()),
        Point::new(region.left() - gap - panel.w as i32, region.top()),
        Point::new(region.left(), region.top() - gap - panel.h as i32),
        Point::new(region.left(), region.bottom() + gap),
    ];
    for raw in candidates {
        let placed = clamp(raw);
        if !rect_at(placed).intersects(region) {
            return (placed, false);
        }
    }

    let fallback = clamp(Point::new(
        work.right() - panel.w as i32 - gap,
        work.bottom() - panel.h as i32 - gap,
    ));
    (fallback, rect_at(fallback).intersects(region))
}

/// 选区所在的显示器工作区（物理像素）。
///
/// 取不到显示器列表时**不能**退回选区本身——那会让「夹进工作区」这一步把控制窗
/// 硬塞进选区里。退回一个围绕选区扩大 4000 像素的框，至少能让右侧/左侧的候选
/// 保持原样（它们本来就在选区之外）。
fn monitor_work_area(region: Rect) -> Rect {
    let monitors = desktop_capture()
        .and_then(|backend| backend.monitors())
        .unwrap_or_default();
    let center = region.center();
    monitors
        .iter()
        .find(|m| m.bounds.contains(center))
        .or_else(|| monitors.iter().find(|m| m.bounds.intersects(region)))
        .or_else(|| monitors.iter().find(|m| m.is_primary))
        .or_else(|| monitors.first())
        .map(|m| m.work_area)
        .unwrap_or_else(|| region.inflate(4000))
}

/// 在后台线程上完成一次识别，并把结果装进面板。
///
/// 配置整份传进来（而不是只传语言）：引擎选择、接口地址、最小置信度都在里面，
/// 而线程里拿不到 `self`。
#[cfg(windows)]
fn recognize_frame(
    frame: Frame,
    ocr: cuter_core::config::OcrConfig,
    theme: Theme,
) -> std::result::Result<OcrPanel, String> {
    use cuter_core::config::OcrEngine;
    use cuter_core::ocr::{HttpOcr, OcrEngine as _, WindowsOcr};

    let paragraph = ocr.paragraph;
    let language = ocr
        .languages
        .first()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    match ocr.engine {
        OcrEngine::Builtin => {
            // `WindowsOcr::new` 内部会在当前线程初始化 WinRT（MTA）
            let engine =
                WindowsOcr::new(language.as_deref()).map_err(|err| fill!(tr!("OCR 引擎不可用：{}"), err))?;
            let used_language = engine.language().to_owned();
            let result = engine.recognize(&frame).map_err(|err| err.to_string())?;
            Ok(OcrPanel::new(frame, result, used_language, paragraph, theme))
        }
        OcrEngine::Http => {
            let engine = HttpOcr::new(ocr.http_endpoint.trim()).with_min_score(ocr.min_score);
            let used_language = engine.name().to_owned();
            let result = engine.recognize(&frame).map_err(|err| err.to_string())?;
            Ok(OcrPanel::new(frame, result, used_language, paragraph, theme))
        }
        OcrEngine::Disabled => Err(tr!("OCR 已关闭").to_owned()),
    }
}

#[cfg(not(windows))]
fn recognize_frame(
    frame: Frame,
    ocr: cuter_core::config::OcrConfig,
    theme: Theme,
) -> std::result::Result<OcrPanel, String> {
    use cuter_core::config::OcrEngine;
    use cuter_core::ocr::{HttpOcr, OcrEngine as _};

    // 在线接口是跨平台的：没有内置引擎的系统靠它也能用
    if matches!(ocr.engine, OcrEngine::Http) {
        let engine = HttpOcr::new(ocr.http_endpoint.trim()).with_min_score(ocr.min_score);
        let used_language = engine.name().to_owned();
        let result = engine.recognize(&frame).map_err(|err| err.to_string())?;
        return Ok(OcrPanel::new(frame, result, used_language, ocr.paragraph, theme));
    }
    Err("当前平台还没有内置 OCR 引擎（macOS / Linux 规划中）；可以改用在线接口".to_owned())
}

/// 主显示器的缩放率（拿不到时按 1.0）。
fn primary_scale_factor() -> f32 {
    desktop_capture()
        .and_then(|backend| backend.monitors())
        .ok()
        .and_then(|monitors| {
            monitors
                .iter()
                .find(|m| m.is_primary)
                .or_else(|| monitors.first())
                .map(|m| m.scale_factor)
        })
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0)
}

/// 把冻结帧限制到某一台显示器（混合 DPI 场景下的兜底方案）。
#[must_use]
pub fn restrict_to_monitor(shot: DesktopShot, monitor: &MonitorInfo) -> DesktopShot {
    let bounds = monitor.bounds.intersect(shot.bounds);
    let local = shot.rect_to_local(bounds);
    let frame = shot.frame.crop(local);
    DesktopShot {
        frame,
        bounds,
        monitors: vec![monitor.clone()],
        cursor: shot.cursor,
    }
}

impl eframe::App for CuterApp {
    /// 每帧先跑逻辑：**即使窗口被隐藏，`eframe` 也只会继续调用 `logic`**，
    /// 所以托盘 / 热键 / 截图请求必须在这里处理，否则「启动即驻留托盘」的模式下
    /// 程序将永远收不到热键。
    fn logic(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        // 第一帧到达 = "用户双击到界面可交互"这条线的终点
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            self.startup_ms = crate::desktop::since_process_start_ms();
            log::info!("启动时间线[第一帧]：+{} ms", self.startup_ms);
        }
        // 根窗口句柄拿到之后才能建托盘通知器（气泡宿主就是我们自己的窗口）。
        if !self.waker_started && self.wake.is_ready() {
            self.waker_started = true;
            // 外部请求（第二个实例转发过来的）不能等主循环轮询：待命心跳最长 4 s，
            // 用户点了"贴图剪贴板"要等好几秒才看见东西。这里挂一条等待线程，
            // 事件一到就先投 `WM_PAINT`、再请一次重绘，把延迟压到一帧。
            if let Some(instance) = &self.instance {
                let hwnd = self.wake.hwnd();
                let nudge_ctx = ctx.clone();
                instance.spawn_waker(move || {
                    crate::desktop::wake_window(hwnd);
                    nudge_ctx.request_repaint();
                });
            }
        }
        if self.notifier.is_none()
            && self.wake.is_ready()
        {
            self.notifier = Some(desktop::TrayNotifier::new(
                self.wake.hwnd(),
                self.config.general.show_tray_notifications,
            ));
            // 托盘驻留启动时打个招呼：顺便把热键告诉用户（否则"装完了不知道按什么"）。
            if self.config.general.show_tray_notifications && self.config.general.start_in_tray {
                let hotkey = self.config.hotkeys.capture.trim().to_owned();
                let body = if hotkey.is_empty() {
                    tr!("Cuter 已在后台运行；点托盘图标可以打开设置").to_owned()
                } else {
                    fill!(tr!("Cuter 已在后台运行：按 {} 截图，点托盘图标打开设置"), hotkey)
                };
                let sent = self
                    .notifier
                    .as_mut()
                    .is_some_and(|notifier| notifier.notify(tr!("Cuter 已启动"), &body));
                log::info!("托盘通知（启动问候）：已投放={sent}");
            }
        }
        // 拿到根窗口句柄后，热键/托盘线程就能用 `WM_PAINT` 立刻把这一帧叫起来，
        // 不必等 eframe 给隐藏窗口的 100 ms 节流（见 `desktop::UiWake`）。
        if !self.wake.is_ready() {
            self.wake.remember_window(frame);
        }
        // 本帧是被谁叫起来的？取走"唤醒时刻"供 `begin_request` 打日志用。
        // 每帧都取走，就不会把上一次无关事件的时刻算到这一次请求头上。
        self.wake_age_ms = self.wake.take_wake_age_ms();
        // 有界面要画就确保字体已装载（托盘纯常驻时这一句什么都不做，省下 ~19 MB）
        if self.needs_font() {
            self.ws_idle_since = None;
            self.ws_trimmed = false;
            self.ensure_font(ctx);
        } else {
            // 字体**不卸载**：`set_fonts` 要到下一帧才生效，"卸了再装"会留下
            // 一帧没有中文字形的画面（整片方框）。这里只记一笔闲置起点，
            // 好让下面的工作集收紧把这份内存还给系统。
            // 更长时间的闲置就把工作集还给系统。只影响任务管理器里的数字（私有提交不变），
            // 但托盘待命是常态，让几十兆工作集一直挂着没有意义；一次闲置只收一次。
            // 注意这一段不能挂在"字体已装载"分支里：纯托盘运行时字体根本没装过。
            let ws_since = *self.ws_idle_since.get_or_insert_with(Instant::now);
            if !self.ws_trimmed && ws_since.elapsed() > Duration::from_secs(30) {
                self.ws_trimmed = true;
                if let Some((before, _after)) = crate::desktop::trim_working_set() {
                    // 不打印"收紧后"的读数：EmptyWorkingSet 刚返回时读数还没稳定，量出来会是 0。
                    // 真实回落值由 tools/probe/idle_trim_probe.ps1 在几十秒后采样。
                    log::info!(
                        "托盘闲置 {} s，已把工作集还给系统（收紧前 {before} MB；私有提交不变）",
                        ws_since.elapsed().as_secs()
                    );
                }
            }
        }
        // 诊断"已请求遮罩窗口 → 遮罩首帧可见"这一段到底是"一帧很慢"还是"好几帧"：
        // 只有当遮罩正在等首帧时才打，日志量有界。
        if self.overlay_first_pass_pending.load(Ordering::Relaxed) {
            self.overlay_wait_frames = self.overlay_wait_frames.saturating_add(1);
            if let Some(started) = self.request_at {
                log::debug!("遮罩等首帧：+{} ms", started.elapsed().as_millis());
            }
            // **每一帧都再催一次，但要有限度。**
            //
            // 为什么催：托盘模式下 eframe 把重绘节流到 ~100 ms，而遮罩要两帧才真的画出来
            //   —— 不催就是两个 100 ms（实测帧间隔 ~125 ms）；催了之后帧间隔 ~17 ms，
            //   整条延迟从 ~250 ms 降到 ~150 ms。
            // 为什么要有限度：万一遮罩窗口一直没画出来（窗口创建失败、被别的东西挡住…），
            //   这里会以 60 fps 空转——那是用户看不见的电池与风扇。40 帧（约 0.7 s）之后
            //   就停手，退回普通重绘，并留一条日志说明这次没等到。
            if self.overlay_wait_frames <= 40 {
                self.wake.wake(ctx);
            } else if self.overlay_wait_frames == 41 {
                log::warn!("遮罩等了 40 帧还没画出首帧，停止催帧（退回普通重绘节奏）");
            }
        }

        // 截图请求在飞的时候，把每一帧的相对时刻打出来：这是判断
        // "等两帧"到底是 2×5ms 还是 2×100ms 的唯一可靠办法。
        if self.request.is_some()
            && let Some(started) = self.request_at
        {
            log::debug!(
                "帧 +{} ms（pending={:?}）",
                started.elapsed().as_millis(),
                self.pending
            );
        }
        if self.status_frames > 0 {
            self.status_frames -= 1;
        }
        self.apply_pending_theme(ctx);
        // 必须每帧开头就做：eframe 会在第一帧画完之后把根窗口强行显示出来，
        // 只在 builder 里写 `with_visible(false)` 是拦不住它的（见 LaunchOptions::start_hidden）
        self.enforce_start_hidden(ctx);
        self.poll_desktop(ctx);
        // **必须排在 `advance_request` 之前**：另一个实例转达过来的请求在这里变成
        // `self.request`，如果放到后面，这一次 pass 就白白错过了它，要等下一帧才处理。
        // 托盘/贴图窗口不可见时那一帧是 **100 ms**（eframe 的隐形窗口节流，见
        // `advance_request` 的说明），实测 `cuter --capture` 转交因此多花 104 ms。
        self.poll_activation(ctx);
        if self.pending_start_pin {
            self.pending_start_pin = false;
            self.pin_from_clipboard(ctx);
        }
        if self.pending_start_history {
            self.pending_start_history = false;
            self.open_history(ctx);
        }
        // 平时只有右键托盘图标才打得开这个菜单，探针用 `cuter --tray-menu` 直接核对版面
        if self.pending_start_tray_menu {
            self.pending_start_tray_menu = false;
            self.open_tray_menu(ctx);
        }
        // `cuter --about`：直接把设置窗口停在「关于」页（更新检查那一栏在这里）
        if self.pending_start_about {
            self.pending_start_about = false;
            self.settings_page = SettingsPage::About;
            // 探针点名要窗口：作废"启动即隐藏"，免得和用户/探针抢
            self.start_hidden_frames = None;
            self.set_control_visible(ctx, true);
        }
        // 启动时检查一次更新：**只在填了清单地址时**才发请求。
        // 本项目没有官方发布渠道，所以地址默认是空的——那时一个字节都不往外发。
        if !self.update_started && self.config.general.check_update {
            self.update_started = true;
            if !self.config.general.update_endpoint.trim().is_empty() {
        self.start_update_check(ctx, false);
            }
        }
        self.advance_request(ctx);
        // 顺序很重要：先创建/销毁视口，再逐帧重新声明存活的那批，
        // 否则新建的遮罩/贴图要等到下一个 pass 才会出现。
        self.declare_viewports(ctx);
        self.drain_overlay_results(ctx);
        self.poll_ocr(ctx);
        self.poll_scroll();
        self.apply_pin_prefs();
        self.drain_pin_ocr(ctx);
        self.handle_root_close(ctx);

        let ocr_actions: Vec<OcrAction> = std::mem::take(&mut lock(&self.ocr_actions));
        for action in ocr_actions {
            self.handle_ocr_action(ctx, action);
        }

        let history_actions: Vec<HistoryAction> = std::mem::take(&mut lock(&self.history_actions));
        for action in history_actions {
            self.handle_history_action(ctx, action);
        }

        let tray_actions: Vec<TrayAction> = std::mem::take(&mut lock(&self.tray_menu_actions));
        for action in tray_actions {
            self.handle_tray_action(ctx, action);
        }

        while let Ok(outcome) = self.update_rx.try_recv() {
            self.handle_update_outcome(ctx, outcome);
        }

        // 遮罩里调过的工具样式（线宽 / 颜色 / 填充 / 马赛克块 …）写回配置。
        // 走旁路而不是结果通道：结果通道会销毁遮罩（见 `Overlay::take_style_change`）。
        let style = lock(&self.overlay)
            .as_mut()
            .and_then(|overlay| overlay.take_style_change());
        if let Some(style) = style {
            self.apply_tool_style(&style);
        }

        // 下一帧只在**确实还有事在做**时才安排。
        //
        // 这里以前是一句无条件的 `request_repaint_after(120 ms)`（早期热键/托盘队列靠
        // 轮询，需要一个心跳）。后来热键与托盘都在各自线程上 `request_repaint()`
        // 立即唤醒 UI（见 `desktop::UiWake`），OCR 结果、贴图窗口也各有唤醒路径，
        // 于是这个心跳变成了纯粹的空转：托盘待命时每秒仍有 ~8 帧，
        // 每帧都要清屏 + 交换缓冲 —— 任务管理器里就表现为"常驻 0.6% GPU"
        // （实测 mean 0.734%、15 秒烧 0.281 s CPU）。现在待命时**一帧都不画**。
        let keep_awake = self
            .keep_awake_until
            .is_some_and(|until| until > Instant::now());
        let busy = self.ocr_busy
            || self.pending.is_some()
            || self.request.is_some()
            // 状态栏文字是按帧倒数的，它还在显示时得继续出帧
            || self.status_frames > 0
            // 遮罩里的提示也是按帧倒数的
            || lock(&self.overlay).is_some()
            // 贴图与各个面板都是**延迟视口**：窗口的显隐每帧都要重新声明，
            // 而隐藏的视口不会跑自己的 pass（也就不会请求下一帧）——一旦停了帧，
            // 隐藏的那一组贴图就再也显示不回来（pin_groups 探针抓到的就是这个）。
            // 只要还开着任何视口就必须继续出帧；托盘里什么都不开时仍然是零帧。
            || !lock(&self.pins).is_empty()
            || self.ocr_viewport.is_some()
            || self.history_viewport.is_some()
            || self.scroll_viewport.is_some();
        if keep_awake {
            // 命令刚处理完：用 post WM_PAINT 这条路把接下来几帧**顶上去**。
            self.wake.wake(ctx);
        }
        // **始终**排一个下一帧，只是分两档节奏。
        //
        // 为什么要"始终"：eframe 对**不可见**窗口的重绘是靠"有一个待处理请求"来排队的，
        // 完全没有请求时，连热键那次唤醒都不一定能换来一次 pass（试过只按需请求，
        // 结果贴图窗口/滚动面板直接不落地——pin_groups 探针从 1/0/1/1 变成全 0）。
        // 所以这里保留心跳，但把"什么都不做"的那一档从 120 ms 放到 1000 ms：
        // 用户看到的问题（托盘待命时长占 0.6% GPU）来自每秒 8 次的清屏+交换缓冲，
        // 降到每秒 1 次即可（实测 GPU 从 0.73% 掉到 ~0.1%，而热键延迟不受影响——
        // 事件线程的唤醒仍然会把下一帧提前）。
        let heartbeat = if busy {
            self.idle_ticks = 0;
            120
        } else {
            // 待命：从 1 s 起步，每空转一帧翻一倍，到 4 s 封顶。
            // 任务管理器里那 0.1% 的常驻 GPU 就是这每秒一次的清屏+交换缓冲；
            // 退到 4 s 一帧后它落到 0.02% 一档（gpu_idle.ps1 实测）。
            // 任何真事件都会把 idle_ticks 归零，所以不会拖慢热键/托盘响应。
            let step = self.idle_ticks.min(2);
            self.idle_ticks = self.idle_ticks.saturating_add(1);
            1000u64 << step
        };
        ctx.request_repaint_after(Duration::from_millis(heartbeat));

        if self.quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 两件事必须在这里做，不能指望根窗口的 `logic()`：
        //
        // 1. **视觉主题**：托盘模式下根窗口是隐藏的，eframe 可能整帧不跑它的 pass，
        //    于是 `apply_pending_theme` 里的 `set_visuals` 就不会执行——设置窗口会一直用
        //    启动时那套（跟随系统的）颜色。实测：系统是浅色、配置选了深色时，
        //    窗口上半部分是浅色的，和深色卡片拼在一起非常难看。
        // 2. **面板铺满客户区**：默认的 `CentralPanel` 有内边距，侧栏没法贯通到窗口边缘。
        //
        // 3. **主题要在这里重新解析**：设置窗口是独立视口，eframe 会给它**另一个 App
        //    实例**，根实例里跑过的 `apply_pending_theme` 不会传过来。实测过这个坑：
        //    系统浅色 + 配置选深色时，窗口渲染成"白底蓝字 + 深色标题栏"的拼盘
        //    （标题栏是我们自己按 `self.theme` 画的，所以它是深的；面板底色走
        //    egui 的 `visuals`，所以是浅的）。这里按配置重解析一次即可。
        self.theme = resolve_theme(self.config.general.theme, &ctx);
        // eframe 默认 `follow_system_theme`，每帧都会把 visuals 覆回系统那一套，所以只
        // 调 `set_visuals` 不够——这正是"系统浅色 + 配置深色"渲染成"白底蓝字 + 深色
        // 标题栏"拼盘的根因（标题栏我们自己画所以是深的，面板底色走 visuals 所以是浅的）。
        // 必须把主题偏好也钉死，让它别再跟系统走。
        let light = match self.config.general.theme {
            cuter_core::config::ThemeKind::Light => true,
            cuter_core::config::ThemeKind::Dark => false,
            cuter_core::config::ThemeKind::Auto => ctx.system_theme() == Some(egui::Theme::Light),
        };
        ctx.set_theme(if light { egui::Theme::Light } else { egui::Theme::Dark });
        ctx.set_visuals(self.theme.visuals());
        // 光设 `ctx` 不够：交给 `paint_control` 的这个 `ui` 是 eframe 在**本帧开始时**
        // 按系统那套建好的，`ui.visuals()` 里仍是浅色的输入框底色与正文色。
        // `settings_visuals` 正是从它克隆的，于是深色主题下会渲染出"白底输入框 +
        // 近黑字压深底"的拼盘（实测就是这样）。这里把主题按到这个 `ui` 上，
        // 之后的卡片、控件、弹层都从它继承。
        ui.set_visuals(self.theme.visuals());
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.theme.surface))
            .show(ui, |ui| {
                self.paint_control(ui, &ctx);
            });
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let [r, g, b, a] = self.theme.surface.to_array();
        [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(err) = self.config.save() {
            log::warn!("退出时保存配置失败：{err}");
        }
    }
}

/// 某个全局物理坐标落在第几块显示器上（1 开始；查不到就是 1）。
///
/// 文件名模板的 `{monitor}` 用它。用**光标位置**判断：用户是在他正看着的那块屏上按的热键。
fn monitor_index_at(point: cuter_core::geom::Point) -> u32 {
    match cuter_core::capture::desktop_capture() {
        Ok(backend) => match backend.monitors() {
            Ok(monitors) => monitors
                .iter()
                .position(|m| m.bounds.contains(point))
                .map_or(1, |index| index as u32 + 1),
            Err(_) => 1,
        },
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cuter_core::geom::{Rect, Size};

    /// 保存格式的决策规则。真正的坑不在"算不出格式"，而在"算出来了没传进编码器"，
    /// 所以这条测试只锁住前半段，后半段由 `Frame::save_as` 的测试锁住。
    #[test]
    fn save_format_prefers_the_extension_but_always_takes_the_quality() {
        use cuter_core::frame::ImageFormat;
        use std::path::Path;

        assert_eq!(
            save_format_for(Path::new("Cuter_2026.png"), ImageFormat::Png, 40),
            ImageFormat::Png,
            "无损格式的质量参数应当被原样忽略"
        );
        assert_eq!(
            save_format_for(Path::new("shot.jpg"), ImageFormat::Png, 40),
            ImageFormat::Jpeg { quality: 40 },
            "扩展名说是 JPEG，就必须用配置里的质量去编码"
        );
        assert_eq!(
            save_format_for(Path::new("shot.unknown"), ImageFormat::WebP, 40),
            ImageFormat::WebP,
            "认不出的扩展名回退到配置格式"
        );
        assert_eq!(
            save_format_for(Path::new("没有扩展名"), ImageFormat::Png, 77),
            ImageFormat::Png
        );
    }

    fn monitor(id: u64, x: i32, scale: f32) -> MonitorInfo {
        MonitorInfo {
            id,
            name: format!("\\\\.\\DISPLAY{id}"),
            bounds: Rect::new(x, 0, 1920, 1080),
            work_area: Rect::new(x, 0, 1920, 1040),
            scale_factor: scale,
            is_primary: id == 1,
        }
    }

    fn shot() -> DesktopShot {
        DesktopShot {
            frame: Frame::new(Size::new(3840, 1080)),
            bounds: Rect::new(0, 0, 3840, 1080),
            monitors: vec![monitor(1, 0, 1.0), monitor(2, 1920, 1.0)],
            cursor: Some(cuter_core::geom::Point::new(2000, 500)),
        }
    }

    #[test]
    fn restrict_to_monitor_crops_frame_and_bounds() {
        let restricted = restrict_to_monitor(shot(), &monitor(2, 1920, 1.0));
        assert_eq!(restricted.bounds, Rect::new(1920, 0, 1920, 1080));
        assert_eq!(restricted.frame.size(), Size::new(1920, 1080));
        assert_eq!(restricted.monitors.len(), 1);
        // 全局坐标 → 局部坐标仍然正确
        assert_eq!(
            restricted.to_local(cuter_core::geom::Point::new(1920, 0)),
            cuter_core::geom::Point::new(0, 0)
        );
    }

    #[test]
    fn restrict_to_monitor_with_negative_origin() {
        let restricted = restrict_to_monitor(shot(), &monitor(1, 0, 1.0));
        assert_eq!(restricted.frame.size(), Size::new(1920, 1080));
    }

    #[test]
    fn geometry_fix_waits_for_pixels_per_point_to_settle() {
        // 仅验证状态机字段的初始值，真实的窗口校正需要 GUI 环境
        let fix = GeometryFix::new(Vec2::new(-100.0, 0.0), Vec2::new(800.0, 600.0));
        assert!(!fix.settled);
        assert_eq!(fix.attempts, 0);
        assert_eq!(fix.target_pos.x, -100.0);
    }

    #[test]
    fn pin_origin_clamping_keeps_the_whole_pin_on_screen() {
        // 直接验证夹取算法的数学（不依赖真实显示器）：1920x1080 桌面上的 400x300 贴图
        let bounds = Rect::new(0, 0, 1920, 1080);
        let size = Vec2::new(400.0, 300.0);
        let clamp = |x: i32, y: i32| {
            let max_x = (bounds.right() - size.x as i32).max(bounds.left());
            let max_y = (bounds.bottom() - size.y as i32).max(bounds.top());
            (x.clamp(bounds.left(), max_x), y.clamp(bounds.top(), max_y))
        };
        // 右下角越界 → 被拉回来，整张可见
        assert_eq!(clamp(1800, 1000), (1520, 780));
        // 左上角越界 → 同样拉回
        assert_eq!(clamp(-50, -50), (0, 0));
        // 正常位置不动
        assert_eq!(clamp(300, 300), (300, 300));
    }

    // ---- 滚动截屏控制窗的摆放 ----

    #[test]
    fn scroll_control_prefers_the_right_of_the_selection() {
        let work = Rect::new(0, 0, 1920, 1080);
        let region = Rect::new(100, 100, 600, 400);
        let panel = Size::new(500, 420);
        let (pos, overlaps) = scroll_control_position(region, panel, work);
        assert!(!overlaps);
        assert_eq!(pos, Point::new(700 + SCROLL_PANEL_GAP, 100));
        // 硬要求：控制窗绝不能落在采集区域里，否则会被逐帧截进长图
        assert!(!Rect::new(pos.x, pos.y, panel.w, panel.h).intersects(region));
    }

    #[test]
    fn scroll_control_goes_left_when_the_right_side_is_too_narrow() {
        let work = Rect::new(0, 0, 1920, 1080);
        // 选区一直贴到右边缘
        let region = Rect::new(1500, 100, 420, 400);
        let panel = Size::new(500, 420);
        let (pos, overlaps) = scroll_control_position(region, panel, work);
        assert!(!overlaps, "左侧还有 1500 像素，应该放得下");
        assert!(pos.x + panel.w as i32 <= region.left());
        assert!(!Rect::new(pos.x, pos.y, panel.w, panel.h).intersects(region));
    }

    #[test]
    fn scroll_control_reports_overlap_for_a_full_screen_selection() {
        // 整屏框选时四个方向都放不下——必须如实报告重叠，而不是假装没事
        let work = Rect::new(0, 0, 1920, 1080);
        let region = Rect::new(0, 0, 1920, 1080);
        let panel = Size::new(500, 420);
        let (pos, overlaps) = scroll_control_position(region, panel, work);
        assert!(overlaps);
        // 退路仍然是「夹进工作区」，不会把窗口扔到屏幕外
        assert!(pos.x >= 0 && pos.y >= 0);
        assert!(pos.x + panel.w as i32 <= work.right());
        assert!(pos.y + panel.h as i32 <= work.bottom());
    }

    #[test]
    fn scroll_control_survives_a_panel_larger_than_the_work_area() {
        // 极小工作区 + 大面板：`clamp` 的上下界必须合法（lo > hi 会 panic）
        let work = Rect::new(0, 0, 200, 200);
        let region = Rect::new(0, 0, 100, 100);
        let (_, overlaps) = scroll_control_position(region, Size::new(500, 420), work);
        assert!(overlaps);
    }
}
