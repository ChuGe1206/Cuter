//! 浮动工具条。
//!
//! 工具条紧随选区出现：默认在选区下方，下方放不下就翻到上方，再放不下就压进
//! 选区内部的底部（全屏框选时最常见的情况）。水平方向始终夹在屏幕内。
//!
//! 所有图标都用 `Painter` **矢量绘制**，不依赖任何图标字体：
//!
//! * 避免引入几百 KB 的图标字体，符合「小体积」目标；
//! * 任意 DPI 下都锐利（虽然遮罩固定 1:1，但设置窗口可能不是）；
//! * 图标与按钮命中区在同一个函数里计算，不会出现「看得见点不着」。

use crate::{fill, tr};

use egui::{
    Align2, Area, Color32, CornerRadius, FontId, Frame, Id, Margin, Order, Pos2, Rect, Sense,
    Stroke, StrokeKind, Vec2,
};

use cuter_core::config::ToolKind;

use crate::annotate::{PALETTE, ToolSettings};
use crate::theme::{Theme, metrics};

/// 工具条上的一次交互。
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarAction {
    SetTool(ToolKind),
    SetColor(Color32),
    SetWidth(f32),
    SetFill(bool),
    /// 工具样式整包变化（颜色 / 线宽 / 填充 / 马赛克块 / 模糊半径 / 字号）。
    ///
    /// 宿主收到后写回配置，这样"上次调好的样子"下次还在——
    /// 之前只有设置窗口里的改动会持久化，遮罩里的调完就丢。
    SetStyle(Box<ToolSettings>),
    Undo,
    Redo,
    Clear,
    Copy,
    Save,
    SaveAs,
    /// 把当前结果贴到屏幕上。
    Pin,
    Ocr,
    /// 用当前选区开始滚动截屏（拼长图）。
    Scroll,
    OpenSettings,
    /// 关闭遮罩（放弃本次截图）。
    Close,
    ToggleMagnifier,
    /// 贴图初始不透明度。
    SetOpacity(f32),
    /// 重新截图（放弃当前冻结帧，重新捕获）。
    Recapture,
    /// 开关窗口边缘吸附。
    ToggleSnap,
}

/// 工具条的输入状态与派生信息。
#[derive(Clone, Debug)]
pub struct ToolbarState {
    pub settings: ToolSettings,
    pub can_undo: bool,
    pub can_redo: bool,
    /// 是否已经框出了选区（没选区时只显示「取消」）。
    pub has_selection: bool,
    /// 有内容正在后台处理（例如 OCR），显示为转圈。
    pub busy: Option<String>,
    pub magnifier: bool,
    /// 窗口边缘吸附是否开启（按住 Alt 可临时关闭）。
    pub snap_enabled: bool,
    pub opacity: f32,
    /// 一次性提示（例如「已复制」），由调用方在若干帧后清除。
    pub status: Option<String>,
    pub show_color_popup: bool,
    pub show_width_popup: bool,
    /// 样式是否被改过（弹窗里拖了滑块/点了预设）：用来在弹窗关闭时整包上报一次。
    pub settings_changed: bool,
    pub show_more_popup: bool,
}

impl Default for ToolbarState {
    fn default() -> Self {
        Self {
            settings: ToolSettings::default(),
            can_undo: false,
            can_redo: false,
            has_selection: true,
            busy: None,
            magnifier: true,
            snap_enabled: true,
            opacity: 0.95,
            status: None,
            show_color_popup: false,
            show_width_popup: false,
        settings_changed: false,
            show_more_popup: false,
        }
    }
}

impl ToolbarState {
    /// 任何弹窗是否打开。
    #[must_use]
    pub fn any_popup_open(&self) -> bool {
        self.show_color_popup || self.show_width_popup || self.show_more_popup
    }

    pub fn close_popups(&mut self) {
        self.show_color_popup = false;
        self.show_width_popup = false;
        self.show_more_popup = false;
    }
}

/// 工具条按钮的图标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Icon {
    Cursor,
    Rectangle,
    Ellipse,
    Arrow,
    Pen,
    Highlighter,
    Mosaic,
    Blur,
    Text,
    Counter,
    Undo,
    Redo,
    Save,
    Copy,
    Pin,
    Scan,
    Scroll,
    Gear,
    Close,
    Trash,
    Magnifier,
    Done,
}

impl Icon {
    fn for_tool(tool: ToolKind) -> Option<Self> {
        Some(match tool {
            ToolKind::Select => Self::Cursor,
            ToolKind::Rectangle => Self::Rectangle,
            ToolKind::Ellipse => Self::Ellipse,
            ToolKind::Arrow => Self::Arrow,
            ToolKind::Pen => Self::Pen,
            ToolKind::Highlighter => Self::Highlighter,
            ToolKind::Mosaic => Self::Mosaic,
            ToolKind::Blur => Self::Blur,
            ToolKind::Text => Self::Text,
            ToolKind::Counter => Self::Counter,
            ToolKind::Laser => return None,
        })
    }
}

/// 分隔符宽度（含左右留白）。
const SEPARATOR: f32 = 9.0;

/// 工具条上「工具」组的按钮数量。
const TOOL_BUTTONS: usize = ToolKind::ALL.len();
/// 颜色 + 线宽 + 更多（设计稿：线宽·颜色·更多）。
const STYLE_BUTTONS: usize = 3;
/// 撤销 + 重做。
const HISTORY_BUTTONS: usize = 2;
/// 完成——全条唯一的实心按钮（保存 / 复制 / 贴图 / OCR / 滚动截屏都收进了「更多」）。
const ACTION_BUTTONS: usize = 1;
/// 工具条总按钮数。
const TOTAL_BUTTONS: usize = TOOL_BUTTONS + STYLE_BUTTONS + HISTORY_BUTTONS + ACTION_BUTTONS;
/// 分组分隔符数量（组 1|2|3 之后各一个）。
const SEPARATORS: usize = 3;

/// 工具条尺寸（用于定位；实际渲染会自然贴合）。
#[must_use]
pub fn desired_size() -> Vec2 {
    let width = metrics::BUTTON * TOTAL_BUTTONS as f32
        + SEPARATOR * SEPARATORS as f32
        + metrics::PADDING * 2.0;
    let height = metrics::BUTTON + metrics::PADDING * 2.0;
    Vec2::new(width, height)
}

/// 根据选区与屏幕范围计算工具条左上角。
#[must_use]
pub fn position(anchor: Rect, size: Vec2, screen: Rect) -> Pos2 {
    let gap = metrics::GAP;

    // 垂直：优先下方 → 上方 → 压进选区内部底部
    let mut y = anchor.bottom() + gap;
    if y + size.y > screen.bottom() {
        let above = anchor.top() - gap - size.y;
        if above >= screen.top() {
            y = above;
        } else if anchor.height() > size.y + gap * 2.0 {
            y = anchor.bottom() - gap - size.y;
        } else {
            y = screen.bottom() - size.y;
        }
    }
    y = y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));

    // 水平：以选区中心为准，夹在屏幕内
    let mut x = anchor.center().x - size.x / 2.0;
    x = x.clamp(screen.left(), (screen.right() - size.x).max(screen.left()));

    Pos2::new(x.round(), y.round())
}

/// 渲染工具条。返回本帧发生的交互（每帧最多一个）。
pub fn show(
    ctx: &egui::Context,
    state: &mut ToolbarState,
    theme: &Theme,
    anchor: Rect,
    screen: Rect,
) -> Option<ToolbarAction> {
    let size = desired_size();
    let origin = position(anchor, size, screen);

    let mut action: Option<ToolbarAction> = None;
    // 记录颜色/线宽/更多按钮的位置，供弹窗与「点击外部关闭」使用
    let mut color_btn: Option<Rect> = None;
    let mut width_btn: Option<Rect> = None;
    let mut more_btn: Option<Rect> = None;

    Area::new(Id::new("cuter-toolbar"))
        .order(Order::Foreground)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            Frame::new()
                .fill(theme.surface.gamma_multiply(0.82))
                .stroke(Stroke::new(1.0, theme.outline))
                .corner_radius(CornerRadius::same(metrics::CORNER))
                .inner_margin(Margin::same(metrics::PADDING as i8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.horizontal(|ui| {
                        // ---- 组 1：标注工具 ----
                        for (index, tool) in ToolKind::ALL.iter().copied().enumerate() {
                            let Some(icon) = Icon::for_tool(tool) else {
                                continue;
                            };
                            let active = state.settings.tool == tool;
                            // 数字键提示写进 tooltip，不占界面（设计稿要求）
                            let tip = format!(
                                "{}（{} {}）",
                                crate::i18n::tr(tool.label()),
                                tr!("数字键"),
                                index + 1
                            );
                            let response = icon_button(ui, theme, icon, active, true, &tip);
                            if response.clicked() {
                                action = Some(ToolbarAction::SetTool(tool));
                            }
                        }

                        separator(ui, theme);

                        // ---- 组 2：颜色与线宽 ----
                        let response = color_button(ui, theme, &state.settings);
                        color_btn = Some(response.rect);
                        if response.clicked() {
                            let open = !state.show_color_popup;
                            state.close_popups();
                            state.show_color_popup = open;
                        }

                        let response = width_button(ui, theme, &state.settings);
                        width_btn = Some(response.rect);
                        if response.clicked() {
                            let open = !state.show_width_popup;
                            state.close_popups();
                            state.show_width_popup = open;
                        }

                        let response = icon_button(ui, theme, Icon::Gear, false, true, tr!("更多"));
                        more_btn = Some(response.rect);
                        if response.clicked() {
                            let open = !state.show_more_popup;
                            state.close_popups();
                            state.show_more_popup = open;
                        }

                        separator(ui, theme);

                        // ---- 组 3：撤销 / 重做 ----
                        let response =
                            icon_button(ui, theme, Icon::Undo, false, state.can_undo, "撤销 (Ctrl+Z)");
                        if response.clicked() {
                            action = Some(ToolbarAction::Undo);
                        }
                        let response =
                            icon_button(ui, theme, Icon::Redo, false, state.can_redo, "重做 (Ctrl+Y)");
                        if response.clicked() {
                            action = Some(ToolbarAction::Redo);
                        }

                        separator(ui, theme);

                        // ---- 组 4：完成（全条唯一的实心按钮）----
                        let busy = state.busy.is_some();
                        let response = done_button(ui, theme, !busy, tr!("完成 (Enter)"));
                        if response.clicked() {
                            action = Some(ToolbarAction::Copy);
                        }

                        if busy && let Some(label) = &state.busy {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(label)
                                    .color(theme.on_surface_dim)
                                    .size(13.0),
                            );
                        }
                    });
                });
        });

    // ---- 弹窗 ----
    let mut popup_rect: Option<Rect> = None;
    if state.show_color_popup
        && let Some(rect) = color_btn
    {
        let (picked, area) = color_popup(ctx, state, theme, rect, screen);
        popup_rect = Some(area);
        if let Some(color) = picked {
            action = Some(ToolbarAction::SetColor(color));
        }
    }
    if state.show_width_popup
        && let Some(rect) = width_btn
    {
        let (picked, area) = tool_options_popup(ctx, state, theme, rect, screen);
        popup_rect = Some(area);
        if let Some(a) = picked {
            action = Some(a);
        }
    }
    if state.show_more_popup
        && let Some(rect) = more_btn
    {
        let (picked, area) = more_popup(ctx, state, theme, rect, screen);
        popup_rect = Some(area);
        if let Some(a) = picked {
            action = Some(a);
        }
    }

    // 点击弹窗与按钮之外的位置关闭弹窗
    if state.any_popup_open() && ctx.input(|i| i.pointer.any_click()) {
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let inside = pointer.is_some_and(|p| {
            [color_btn, width_btn, more_btn]
                .into_iter()
                .flatten()
                .any(|r| r.expand(2.0).contains(p))
                || popup_rect.is_some_and(|r| r.contains(p))
        });
        if !inside {
            state.close_popups();
        }
    }

    // 状态提示条（画在工具条上方，不参与宽度计算）
    if let Some(status) = state.status.clone() {
        status_label(ctx, theme, origin, &status);
    }

    action
}

// ---------------------------------------------------------------------------
// 控件
// ---------------------------------------------------------------------------

fn icon_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: Icon,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(metrics::BUTTON), Sense::click());
    let hovered = enabled && response.hovered();

    let background = if active {
        theme.accent_subtle
    } else if hovered {
        theme.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    if background != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(
                rect.shrink(1.0),
                CornerRadius::same(metrics::CORNER_BUTTON),
                background,
            );
    }

    let color = if !enabled {
        theme.on_surface_dim
    } else if active {
        theme.accent
    } else {
        theme.on_surface
    };
    draw_icon(ui.painter(), rect.shrink(8.0), icon, color);

    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// 工具条上唯一的实心按钮：完成（复制并退出）。
fn done_button(ui: &mut egui::Ui, theme: &Theme, enabled: bool, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(metrics::BUTTON), Sense::click());
    let hovered = enabled && response.hovered();

    let fill = if !enabled {
        theme.surface_active
    } else if hovered {
        theme.accent_hover
    } else {
        theme.accent
    };
    ui.painter().rect_filled(
        rect.shrink(1.0),
        CornerRadius::same(metrics::CORNER_BUTTON),
        fill,
    );
    draw_icon(ui.painter(), rect.shrink(8.0), Icon::Done, Color32::WHITE);

    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

fn color_button(ui: &mut egui::Ui, theme: &Theme, settings: &ToolSettings) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(metrics::BUTTON), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), CornerRadius::same(6), theme.surface_hover);
    }
    let swatch = rect.shrink(8.0);
    ui.painter()
        .rect_filled(swatch, CornerRadius::same(4), settings.color);
    ui.painter().rect_stroke(
        swatch,
        CornerRadius::same(4),
        Stroke::new(1.0, theme.outline),
        StrokeKind::Inside,
    );
    response.on_hover_text(tr!("颜色"))
}

fn width_button(ui: &mut egui::Ui, theme: &Theme, settings: &ToolSettings) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(metrics::BUTTON), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), CornerRadius::same(6), theme.surface_hover);
    }
    // 一条粗细等于当前线宽的横线，直观表达「现在多粗」
    let thickness = settings.stroke_width.clamp(1.0, 12.0);
    let center = rect.center();
    ui.painter().rect_filled(
        Rect::from_center_size(center, Vec2::new(rect.width() - 16.0, thickness)),
        CornerRadius::same(2),
        theme.on_surface,
    );
    response.on_hover_text(fill!(tr!("线宽 {} px"), format!("{:.0}", settings.stroke_width)))
}

fn separator(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(SEPARATOR, metrics::BUTTON), Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        egui::Rangef::new(rect.top() + 7.0, rect.bottom() - 7.0),
        Stroke::new(1.0, theme.outline),
    );
}

fn popup_frame(theme: &Theme) -> Frame {
    Frame::new()
        .fill(theme.surface)
        .stroke(Stroke::new(1.0, theme.outline))
        .corner_radius(CornerRadius::same(metrics::CORNER))
        .inner_margin(Margin::same(8))
}

fn popup_pos(button: Rect, size: Vec2, screen: Rect) -> Pos2 {
    let y = if button.top() - 4.0 - size.y >= screen.top() {
        button.top() - 4.0 - size.y
    } else {
        button.bottom() + 4.0
    };
    let x = button
        .left()
        .clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    Pos2::new(x.round(), y.round())
}

/// 取色弹窗。返回 `(选中的颜色, 弹窗实际矩形)`。
fn color_popup(
    ctx: &egui::Context,
    state: &mut ToolbarState,
    theme: &Theme,
    button: Rect,
    screen: Rect,
) -> (Option<Color32>, Rect) {
    let size = Vec2::new(232.0, 128.0);
    let mut picked = None;
    let area = Area::new(Id::new("cuter-toolbar-popup"))
        .order(Order::Foreground)
        .fixed_pos(popup_pos(button, size, screen))
        .show(ctx, |ui| {
            popup_frame(theme).show(ui, |ui| {
                ui.set_width(size.x - 16.0);
                let swatch = Vec2::splat(24.0);
                for row in 0..2 {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                        for col in 0..8 {
                            let color = PALETTE[row * 8 + col];
                            let (rect, response) = ui.allocate_exact_size(swatch, Sense::click());
                            let selected = state.settings.color == color;
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(4), color);
                            ui.painter().rect_stroke(
                                rect,
                                CornerRadius::same(4),
                                Stroke::new(
                                    if selected { 2.0 } else { 1.0 },
                                    if selected { theme.accent } else { theme.outline },
                                ),
                                StrokeKind::Inside,
                            );
                            if response.clicked() {
                                picked = Some(color);
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                // 完整取色器：任意 sRGB 颜色
                ui.horizontal(|ui| {
                    let mut rgba = state.settings.color.to_array();
                    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                        picked = Some(Color32::from_rgba_unmultiplied(
                            rgba[0], rgba[1], rgba[2], 255,
                        ));
                    }
                    ui.label(
                        egui::RichText::new(hex_string(state.settings.color))
                            .color(theme.on_surface_dim)
                            .monospace()
                            .size(12.0),
                    );
                });
            });
        });
    (picked, area.response.rect)
}

/// 当前工具的选项弹窗（线宽 / 填充 / 马赛克块 …）。
///
/// ## 为什么是"当前工具的选项"，而不是一个固定的"线宽"面板
///
/// 原来这里是个两行小面板：第一行线宽滑块，第二行「填充图形 + 马赛克块大小」——
/// 三个互不相干的东西挤在一个叫「线宽」的弹窗里，用户反馈"很不友好、不像现代程序"
/// （见 HANDOFF 第 20 节）。现在按**当前工具**决定内容：
///
/// * 画线类的工具（矩形/椭圆/箭头/画笔/荧光/序号）→ 线宽预设 + 滑块 + **实时预览**；
///   矩形/椭圆再多一个「填充图形」；
/// * 马赛克 → 块大小 + 一块**效果预览**（直接看到块多大）；
/// * 模糊 → 半径 + 预览；
/// * 文字 → 字号；
/// * 选择工具 → 提示"先在图上拖出选区"。
///
/// 顺带把「改了就丢」修掉：这些改动现在会作为 [`ToolbarAction::SetStyle`] 上报，
/// 由宿主写回配置——下次打开遮罩还是这套颜色/线宽/块大小。
fn tool_options_popup(
    ctx: &egui::Context,
    state: &mut ToolbarState,
    theme: &Theme,
    button: Rect,
    screen: Rect,
) -> (Option<ToolbarAction>, Rect) {
    let tool = state.settings.tool;
    let size = tool_options_size(tool);
    let mut action = None;
    // 改过的样式在弹窗结束时整包上报一次（避免每拖一下写一次配置）
    let before = state.settings.clone();

    let area = Area::new(Id::new("cuter-toolbar-popup"))
        .order(Order::Foreground)
        .fixed_pos(popup_pos(button, size, screen))
        .show(ctx, |ui| {
            popup_frame(theme).show(ui, |ui| {
                ui.set_width(size.x - 16.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tool.label()).color(theme.on_surface).size(13.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(tool_options_value_text(&state.settings))
                                .color(theme.on_surface_dim)
                                .size(11.0),
                        );
                    });
                });
                ui.add_space(4.0);

                match tool {
                    ToolKind::Mosaic => {
                        ui.add(
                            egui::Slider::new(&mut state.settings.mosaic_block, 2..=64)
                                .fixed_decimals(0)
                                .suffix(" px"),
                        );
                        mosaic_preview(ui, theme, state.settings.mosaic_block);
                    }
                    ToolKind::Blur => {
                        ui.add(
                            egui::Slider::new(&mut state.settings.blur_radius, 1..=40)
                                .fixed_decimals(0)
                                .suffix(" px"),
                        );
                        blur_preview(ui, theme, state.settings.blur_radius);
                    }
                    ToolKind::Text => {
                        ui.add(
                            egui::Slider::new(&mut state.settings.font_size, 8.0..=96.0)
                                .fixed_decimals(0)
                                .suffix(" px"),
                        );
                    }
                    ToolKind::Select => {
                        ui.label(
                            egui::RichText::new(tr!("在图上拖出一块选区，之后这里就是标注工具了"))
                                .color(theme.on_surface_dim)
                                .size(11.0),
                        );
                    }
                    _ => {
                        // 线宽：预设圆点 + 滑块 + 预览
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                            for preset in WIDTH_PRESETS {
                                let (rect, response) =
                                    ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                                let selected =
                                    (state.settings.stroke_width - preset).abs() < 0.5;
                                let center = rect.center();
                                let radius = (preset * 0.5 + 2.0).clamp(2.0, 8.0);
                                ui.painter().circle_filled(
                                    center,
                                    radius,
                                    if selected {
                                        theme.accent
                                    } else {
                                        theme.on_surface_dim
                                    },
                                );
                                if selected {
                                    ui.painter().circle_stroke(
                                        center,
                                        radius + 3.0,
                                        Stroke::new(1.5, theme.accent),
                                    );
                                }
                                if response.clicked() {
                                    state.settings.stroke_width = preset;
                                }
                                response.on_hover_text(format!("{} px", preset as u32));
                            }
                            ui.add(
                                egui::Slider::new(&mut state.settings.stroke_width, 1.0..=32.0)
                                    .fixed_decimals(0)
                                    .suffix(" px"),
                            );
                        });
                        ui.add_space(2.0);
                        width_preview(ui, theme, &state.settings);
                        if matches!(tool, ToolKind::Rectangle | ToolKind::Ellipse) {
                            ui.add_space(2.0);
                            ui.checkbox(&mut state.settings.fill_shapes, tr!("填充图形"));
                        }
                    }
                }
            });
        });

    if state.settings != before {
        state.settings_changed = true;
    }
    if state.settings_changed {
        action = Some(ToolbarAction::SetStyle(Box::new(state.settings.clone())));
        state.settings_changed = false;
    }
    (action, area.response.rect)
}

/// 线宽预设（点一下就用，不用拖滑块试）。
const WIDTH_PRESETS: [f32; 4] = [1.0, 2.0, 3.0, 5.0];

fn tool_options_size(tool: ToolKind) -> Vec2 {
    match tool {
        ToolKind::Mosaic | ToolKind::Blur | ToolKind::Text => Vec2::new(248.0, 78.0),
        ToolKind::Select => Vec2::new(248.0, 66.0),
        ToolKind::Rectangle | ToolKind::Ellipse => Vec2::new(292.0, 130.0),
        _ => Vec2::new(292.0, 104.0),
    }
}

fn tool_options_value_text(settings: &ToolSettings) -> String {
    match settings.tool {
        ToolKind::Mosaic => format!("{} px", settings.mosaic_block),
        ToolKind::Blur => format!("{} px", settings.blur_radius),
        ToolKind::Text => format!("{} px", settings.font_size as u32),
        ToolKind::Select => String::new(),
        _ => format!("{} px", settings.stroke_width as u32),
    }
}

/// 用当前颜色与线宽画一条预览线——"3 px 到底多粗"不该靠想象。
fn width_preview(ui: &mut egui::Ui, theme: &Theme, settings: &ToolSettings) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 18.0),
        Sense::hover(),
    );
    let y = rect.center().y;
    ui.painter().line_segment(
        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
        Stroke::new(settings.stroke_width.max(1.0), settings.color),
    );
    if settings.fill_shapes && matches!(settings.tool, ToolKind::Rectangle | ToolKind::Ellipse) {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(0.0, 6.0)),
            CornerRadius::same(2),
            Color32::from_rgba_unmultiplied(
                settings.color.r(),
                settings.color.g(),
                settings.color.b(),
                64,
            ),
        );
    }
    let _ = theme;
}

/// 马赛克效果预览：画一块按块大小分格的色块。
fn mosaic_preview(ui: &mut egui::Ui, theme: &Theme, block: u32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let cells = (rect.width() / (block.max(2) as f32)).clamp(4.0, 40.0) as u32;
    let cell = rect.width() / cells as f32;
    let shades = [
        theme.on_surface_dim,
        theme.outline,
        theme.surface_hover,
        theme.on_surface,
    ];
    for i in 0..cells {
        for j in 0..2 {
            let r = Rect::from_min_size(
                Pos2::new(rect.left() + i as f32 * cell, rect.top() + j as f32 * cell),
                Vec2::new(cell - 1.0, cell - 1.0),
            );
            let shade = shades[((i + j * 3) % shades.len() as u32) as usize];
            ui.painter()
                .rect_filled(r, CornerRadius::ZERO, shade.gamma_multiply(0.55));
        }
    }
}

/// 模糊预览：一条由粗到细的渐变带，接近模糊半径的观感。
fn blur_preview(ui: &mut egui::Ui, theme: &Theme, radius: u32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let steps = 12;
    for i in 0..steps {
        let t = i as f32 / (steps - 1) as f32;
        let x = rect.left() + t * rect.width();
        let alpha = (1.0 - t).powf(1.0 + radius as f32 / 12.0);
        ui.painter().rect_filled(
            Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(rect.width() / steps as f32 + 1.0, rect.height())),
            CornerRadius::ZERO,
            theme.on_surface_dim.gamma_multiply(alpha * 0.7),
        );
    }
}

/// 「更多」弹窗。返回 `(动作, 弹窗矩形)`。
fn more_popup(
    ctx: &egui::Context,
    state: &mut ToolbarState,
    theme: &Theme,
    button: Rect,
    screen: Rect,
) -> (Option<ToolbarAction>, Rect) {
    let size = Vec2::new(190.0, 400.0);
    let mut action = None;
    let area = Area::new(Id::new("cuter-toolbar-popup"))
        .order(Order::Foreground)
        .fixed_pos(popup_pos(button, size, screen))
        .show(ctx, |ui| {
            popup_frame(theme).show(ui, |ui| {
                ui.set_width(size.x - 16.0);
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);

                // 从工具条上收进来的常用动作，快捷键写在标签里
                if menu_row(ui, theme, Icon::Save, tr!("保存 (Ctrl+S)")).clicked() {
                    action = Some(ToolbarAction::Save);
                }
                if menu_row(ui, theme, Icon::Copy, tr!("复制并完成 (Enter)")).clicked() {
                    action = Some(ToolbarAction::Copy);
                }
                if menu_row(ui, theme, Icon::Pin, tr!("贴到屏幕上 (Ctrl+T)")).clicked() {
                    action = Some(ToolbarAction::Pin);
                }
                if menu_row(ui, theme, Icon::Scan, tr!("识别文字 (Ctrl+O)")).clicked() {
                    action = Some(ToolbarAction::Ocr);
                }
                if menu_row(ui, theme, Icon::Scroll, tr!("滚动截屏 (Alt+S)")).clicked() {
                    action = Some(ToolbarAction::Scroll);
                }

                ui.separator();
                if menu_row(ui, theme, Icon::Save, tr!("另存为…")).clicked() {
                    action = Some(ToolbarAction::SaveAs);
                }
                if menu_row(ui, theme, Icon::Trash, tr!("清空标注")).clicked() {
                    action = Some(ToolbarAction::Clear);
                }
                if menu_row(ui, theme, Icon::Magnifier, tr!("重新截图")).clicked() {
                    action = Some(ToolbarAction::Recapture);
                }
                if menu_row(ui, theme, Icon::Scan, tr!("放大镜/取色开关")).clicked() {
                    action = Some(ToolbarAction::ToggleMagnifier);
                }
                // 吸附开关用勾选态呈现——这是用户判断"现在到底吸不吸"的唯一依据
                let snap_label = if state.snap_enabled {
                    tr!("窗口吸附　（Alt 临时关闭）")
                } else {
                    tr!("窗口吸附　（已关闭）")
                };
                let response = menu_row_with_check(ui, theme, Icon::Cursor, snap_label, state.snap_enabled);
                if response.clicked() {
                    action = Some(ToolbarAction::ToggleSnap);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tr!("贴图不透明度"))
                            .color(theme.on_surface_dim)
                            .size(12.0),
                    );
                    if ui
                        .add(
                            egui::Slider::new(&mut state.opacity, 0.1..=1.0)
                                .fixed_decimals(2)
                                .show_value(false),
                        )
                        .changed()
                    {
                        action = Some(ToolbarAction::SetOpacity(state.opacity));
                    }
                });

                ui.separator();
                if menu_row(ui, theme, Icon::Gear, tr!("设置…")).clicked() {
                    action = Some(ToolbarAction::OpenSettings);
                }
                if menu_row(ui, theme, Icon::Close, tr!("退出截图")).clicked() {
                    action = Some(ToolbarAction::Close);
                }
            });
        });
    (action, area.response.rect)
}

fn menu_row(ui: &mut egui::Ui, theme: &Theme, icon: Icon, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 26.0),

        Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), theme.surface_hover);
    }
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 13.0, rect.center().y),
        Vec2::splat(16.0),
    );
    draw_icon(ui.painter(), icon_rect, icon, theme.on_surface);
    ui.painter().text(
        Pos2::new(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        theme.on_surface,
    );
    response
}

/// 带勾选态的菜单行：右侧画一个对勾。
fn menu_row_with_check(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: Icon,
    label: &str,
    checked: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 26.0),
        Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), theme.surface_hover);
    }
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 13.0, rect.center().y),
        Vec2::splat(16.0),
    );
    draw_icon(ui.painter(), icon_rect, icon, theme.on_surface);
    ui.painter().text(
        Pos2::new(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        theme.on_surface,
    );
    if checked {
        // 一个不用额外图标变体就能画出来的对勾
        let mark = Rect::from_center_size(
            Pos2::new(rect.right() - 14.0, rect.center().y),
            Vec2::splat(12.0),
        );
        ui.painter().add(egui::Shape::line(
            vec![
                Pos2::new(mark.left(), mark.center().y),
                Pos2::new(mark.left() + mark.width() * 0.38, mark.bottom()),
                Pos2::new(mark.right(), mark.top()),
            ],
            Stroke::new(2.0, theme.accent),
        ));
    }
    response
}

fn status_label(ctx: &egui::Context, theme: &Theme, anchor: Pos2, text: &str) {
    Area::new(Id::new("cuter-toolbar-status"))
        .order(Order::Foreground)
        .fixed_pos(Pos2::new(anchor.x, anchor.y - 30.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(theme.label_bg)
                .stroke(Stroke::new(1.0, theme.outline))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .color(theme.label_fg)
                            .size(13.0),
                    );
                });
        });
}

#[must_use]
fn hex_string(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

// ---------------------------------------------------------------------------
// 矢量图标
// ---------------------------------------------------------------------------

/// 把单位坐标（0..1）映射到 `rect` 内。
fn at(rect: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(
        rect.min.x + x * rect.width(),
        rect.min.y + y * rect.height(),
    )
}

fn draw_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    let thin = Stroke::new(1.3, color);

    match icon {
        Icon::Done => {
            // 完成：一条勾，比普通图标粗一点
            let check = Stroke::new(2.2, color);
            painter.line_segment([at(rect, 0.20, 0.54), at(rect, 0.42, 0.76)], check);
            painter.line_segment([at(rect, 0.42, 0.76), at(rect, 0.80, 0.26)], check);
        }
        Icon::Cursor => {
            // 鼠标指针：经典箭头轮廓
            let points: Vec<Pos2> = [
                (0.26, 0.12),
                (0.26, 0.80),
                (0.42, 0.64),
                (0.53, 0.90),
                (0.66, 0.84),
                (0.55, 0.59),
                (0.76, 0.56),
            ]
            .iter()
            .map(|(x, y)| at(rect, *x, *y))
            .collect();
            painter.add(egui::Shape::convex_polygon(
                points,
                color,
                Stroke::NONE,
            ));
        }
        Icon::Rectangle => {
            painter.rect_stroke(
                Rect::from_min_max(at(rect, 0.18, 0.22), at(rect, 0.82, 0.78)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
        }
        Icon::Ellipse => {
            let center = rect.center();
            painter.add(egui::Shape::ellipse_stroke(
                center,
                Vec2::new(rect.width() * 0.32, rect.height() * 0.28),
                stroke,
            ));
        }
        Icon::Arrow => {
            let from = at(rect, 0.18, 0.82);
            let to = at(rect, 0.82, 0.18);
            painter.line_segment([from, to], stroke);
            painter.line_segment([to, at(rect, 0.82 - 0.26, 0.18 + 0.04)], stroke);
            painter.line_segment([to, at(rect, 0.82 - 0.04, 0.18 + 0.26)], stroke);
        }
        Icon::Pen => {
            // 斜置的铅笔：笔身 + 笔尖
            painter.line_segment([at(rect, 0.22, 0.78), at(rect, 0.70, 0.30)], stroke);
            painter.line_segment([at(rect, 0.30, 0.86), at(rect, 0.78, 0.38)], stroke);
            painter.add(egui::Shape::convex_polygon(
                vec![at(rect, 0.18, 0.90), at(rect, 0.30, 0.86), at(rect, 0.22, 0.78)],
                color,
                Stroke::NONE,
            ));
            painter.line_segment([at(rect, 0.66, 0.22), at(rect, 0.80, 0.36)], stroke);
        }
        Icon::Highlighter => {
            // 荧光笔：粗笔身 + 下方涂抹带
            painter.line_segment([at(rect, 0.28, 0.62), at(rect, 0.66, 0.24)], Stroke::new(5.0, color));
            painter.line_segment([at(rect, 0.34, 0.70), at(rect, 0.72, 0.32)], stroke);
            painter.rect_filled(
                Rect::from_min_max(at(rect, 0.16, 0.82), at(rect, 0.84, 0.90)),
                CornerRadius::same(1),
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 130),
            );
        }
        Icon::Mosaic => {
            // 3x3 棋盘，暗示像素化
            let cell = rect.width() / 3.0 - 1.0;
            for row in 0..3 {
                for col in 0..3 {
                    if (row + col) % 2 == 0 {
                        continue;
                    }
                    let min = Pos2::new(
                        rect.min.x + col as f32 * (cell + 1.5) + 0.5,
                        rect.min.y + row as f32 * (cell + 1.5) + 0.5,
                    );
                    painter.rect_filled(
                        Rect::from_min_size(min, Vec2::splat(cell)),
                        CornerRadius::ZERO,
                        color,
                    );
                }
            }
            painter.rect_stroke(rect, CornerRadius::same(2), thin, StrokeKind::Inside);
        }
        Icon::Blur => {
            // 水滴
            let center = Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.62);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    at(rect, 0.5, 0.14),
                    at(rect, 0.78, 0.58),
                    at(rect, 0.22, 0.58),
                ],
                color,
                Stroke::NONE,
            ));
            painter.circle_filled(center, rect.width() * 0.28, color);
        }
        Icon::Text => {
            painter.line_segment([at(rect, 0.22, 0.24), at(rect, 0.78, 0.24)], stroke);
            painter.line_segment([at(rect, 0.5, 0.24), at(rect, 0.5, 0.80)], stroke);
            painter.line_segment([at(rect, 0.34, 0.80), at(rect, 0.66, 0.80)], stroke);
        }
        Icon::Counter => {
            painter.add(egui::Shape::ellipse_stroke(
                rect.center(),
                Vec2::new(rect.width() * 0.32, rect.height() * 0.32),
                stroke,
            ));
            // 数字「1」
            painter.line_segment([at(rect, 0.44, 0.38), at(rect, 0.54, 0.30)], thin);
            painter.line_segment([at(rect, 0.54, 0.30), at(rect, 0.54, 0.70)], thin);
        }
        Icon::Undo => {
            painter.add(egui::Shape::line(
                vec![at(rect, 0.24, 0.62), at(rect, 0.34, 0.34), at(rect, 0.62, 0.34), at(rect, 0.72, 0.56)],
                stroke,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![at(rect, 0.14, 0.52), at(rect, 0.38, 0.52), at(rect, 0.26, 0.74)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Redo => {
            painter.add(egui::Shape::line(
                vec![at(rect, 0.76, 0.62), at(rect, 0.66, 0.34), at(rect, 0.38, 0.34), at(rect, 0.28, 0.56)],
                stroke,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![at(rect, 0.86, 0.52), at(rect, 0.62, 0.52), at(rect, 0.74, 0.74)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Save => {
            // 向下的箭头 + 托盘
            painter.line_segment([at(rect, 0.5, 0.16), at(rect, 0.5, 0.62)], stroke);
            painter.add(egui::Shape::convex_polygon(
                vec![at(rect, 0.30, 0.50), at(rect, 0.70, 0.50), at(rect, 0.5, 0.76)],
                color,
                Stroke::NONE,
            ));
            painter.line_segment([at(rect, 0.20, 0.86), at(rect, 0.80, 0.86)], stroke);
        }
        Icon::Copy => {
            painter.rect_stroke(
                Rect::from_min_max(at(rect, 0.16, 0.16), at(rect, 0.62, 0.62)),
                CornerRadius::same(2),
                thin,
                StrokeKind::Middle,
            );
            painter.rect_stroke(
                Rect::from_min_max(at(rect, 0.38, 0.38), at(rect, 0.84, 0.84)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
        }
        Icon::Pin => {
            // 图钉：针杆 + 圆头
            painter.circle_filled(
                Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.32),
                rect.width() * 0.22,
                color,
            );
            painter.line_segment([at(rect, 0.5, 0.50), at(rect, 0.5, 0.86)], stroke);
            painter.line_segment([at(rect, 0.26, 0.50), at(rect, 0.74, 0.50)], stroke);
        }
        Icon::Scan => {
            // 四角取景框 + 文字线
            let len = 0.26;
            for (cx, cy) in [(0.14, 0.14), (0.86, 0.14), (0.14, 0.86), (0.86, 0.86)] {
                let sx = if cx < 0.5 { 1.0 } else { -1.0 };
                let sy = if cy < 0.5 { 1.0 } else { -1.0 };
                painter.line_segment(
                    [at(rect, cx, cy), at(rect, cx + len * sx, cy)],
                    stroke,
                );
                painter.line_segment(
                    [at(rect, cx, cy), at(rect, cx, cy + len * sy)],
                    stroke,
                );
            }
            painter.line_segment([at(rect, 0.34, 0.44), at(rect, 0.66, 0.44)], thin);
            painter.line_segment([at(rect, 0.34, 0.58), at(rect, 0.56, 0.58)], thin);
        }
        Icon::Gear => {
            let center = rect.center();
            let outer = rect.width() * 0.34;
            let inner = rect.width() * 0.14;
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let dir = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment(
                    [center + dir * inner, center + dir * outer],
                    Stroke::new(2.2, color),
                );
            }
            painter.circle_stroke(center, inner, thin);
        }
        Icon::Close => {
            painter.line_segment([at(rect, 0.24, 0.24), at(rect, 0.76, 0.76)], stroke);
            painter.line_segment([at(rect, 0.76, 0.24), at(rect, 0.24, 0.76)], stroke);
        }
        Icon::Trash => {
            painter.line_segment([at(rect, 0.18, 0.28), at(rect, 0.82, 0.28)], stroke);
            painter.rect_stroke(
                Rect::from_min_max(at(rect, 0.26, 0.28), at(rect, 0.74, 0.86)),
                CornerRadius::same(2),
                thin,
                StrokeKind::Middle,
            );
            painter.line_segment([at(rect, 0.42, 0.18), at(rect, 0.58, 0.18)], thin);
        }
        Icon::Scroll => {
            // 一张向下延伸的长纸 + 底部续接箭头：直观表达「一直往下拼」
            painter.rect_stroke(
                Rect::from_min_max(at(rect, 0.30, 0.12), at(rect, 0.70, 0.58)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            painter.line_segment([at(rect, 0.30, 0.72), at(rect, 0.70, 0.72)], thin);
            painter.line_segment([at(rect, 0.30, 0.82), at(rect, 0.70, 0.82)], thin);
            painter.add(egui::Shape::convex_polygon(
                vec![at(rect, 0.42, 0.62), at(rect, 0.58, 0.62), at(rect, 0.50, 0.78)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Magnifier => {
            painter.circle_stroke(
                Pos2::new(rect.min.x + rect.width() * 0.42, rect.min.y + rect.height() * 0.42),
                rect.width() * 0.24,
                stroke,
            );
            painter.line_segment([at(rect, 0.60, 0.60), at(rect, 0.84, 0.84)], stroke);
        }
    }
}

/// 供测试与调试：单位坐标映射的可预测性。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desired_size_matches_button_count() {
        let size = desired_size();
        let expected = metrics::BUTTON * TOTAL_BUTTONS as f32
            + SEPARATOR * SEPARATORS as f32
            + metrics::PADDING * 2.0;
        assert!((size.x - expected).abs() < 0.01);
        assert!((size.y - (metrics::BUTTON + metrics::PADDING * 2.0)).abs() < 0.01);
    }

    #[test]
    fn toolbar_prefers_below_the_selection() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let anchor = Rect::from_min_size(Pos2::new(400.0, 200.0), Vec2::new(600.0, 300.0));
        let pos = position(anchor, desired_size(), screen);
        assert!(pos.y >= anchor.bottom());
    }

    #[test]
    fn toolbar_flips_above_when_no_room_below() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let anchor = Rect::from_min_size(Pos2::new(400.0, 900.0), Vec2::new(600.0, 170.0));
        let pos = position(anchor, desired_size(), screen);
        assert!(pos.y + desired_size().y <= anchor.top(), "应翻到选区上方");
    }

    #[test]
    fn toolbar_stays_inside_screen_for_fullscreen_selection() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let anchor = screen;
        let pos = position(anchor, desired_size(), screen);
        assert!(pos.y >= screen.top());
        assert!(pos.y + desired_size().y <= screen.bottom() + 0.5);
        assert!(pos.x >= screen.left());
        assert!(pos.x + desired_size().x <= screen.right() + 0.5);
    }

    #[test]
    fn toolbar_clamps_horizontally_for_edge_selection() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let anchor = Rect::from_min_size(Pos2::new(1900.0, 100.0), Vec2::new(20.0, 20.0));
        let pos = position(anchor, desired_size(), screen);
        assert!(pos.x + desired_size().x <= screen.right() + 0.5);
    }

    #[test]
    fn negative_origin_screen_is_handled() {
        // 虚拟桌面原点可能落在负半轴（最左侧显示器位于主屏左侧）
        let screen = Rect::from_min_size(Pos2::new(-1920.0, 0.0), Vec2::new(3840.0, 1080.0));
        let anchor = Rect::from_min_size(Pos2::new(-1800.0, 100.0), Vec2::new(300.0, 200.0));
        let pos = position(anchor, desired_size(), screen);
        assert!(pos.x >= screen.left());
        assert!(pos.x + desired_size().x <= screen.right() + 0.5);
    }

    #[test]
    fn hex_string_is_uppercase_six_digits() {
        assert_eq!(hex_string(Color32::from_rgb(0, 145, 255)), "#0091FF");
        assert_eq!(hex_string(Color32::BLACK), "#000000");
    }

    #[test]
    fn every_paintable_tool_has_an_icon() {
        for tool in ToolKind::ALL {
            assert!(
                Icon::for_tool(tool).is_some(),
                "工具 {:?} 缺少图标，工具条会算错宽度",
                tool
            );
        }
        assert_eq!(ToolKind::ALL.len(), TOOL_BUTTONS);
    }

    #[test]
    fn popup_flips_inside_screen_when_button_is_near_top() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let button = Rect::from_min_size(Pos2::new(10.0, 2.0), Vec2::splat(32.0));
        let pos = popup_pos(button, Vec2::new(200.0, 120.0), screen);
        assert!(pos.y >= button.bottom());
    }

    #[test]
    fn popup_clamps_to_right_edge() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let button = Rect::from_min_size(Pos2::new(980.0, 400.0), Vec2::splat(32.0));
        let size = Vec2::new(232.0, 128.0);
        let pos = popup_pos(button, size, screen);
        assert!(pos.x + size.x <= screen.right() + 0.5);
    }
}
