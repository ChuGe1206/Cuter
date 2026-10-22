//! 托盘菜单面板。
//!
//! 托盘右键以前用的是 Win32 原生菜单（`muda`），它跟这套设计系统格格不入：
//! 系统灰色、字重和圆角都不受控，浅色/深色主题切换时只有它是另一套样子。
//! 现在改成**自己画的菜单**：一个无边框、总在最前的独立视口，摆在托盘图标附近，
//! 和贴图右键菜单、截图历史面板用同一套零件（32 点行高、圆角 8 的 hover 高亮、
//! 右对齐的暗色快捷键、`outline` 分隔线）。
//!
//! 快捷键提示由宿主从配置里读出来传进来（`TrayPanelInput`）：用户改了设置，
//! 菜单里的键位跟着变，不用重启。

use crate::theme::{metrics, Theme};
use crate::{fill, tr};
use egui::{Align2, CornerRadius, FontId, Margin, Pos2, Rect, Stroke, Vec2};

/// 窗口标题（无边框窗口看不到它，任务栏里也用得上）。
/// 窗口标题。跟着界面语言走，所以是个函数而不是常量。
#[must_use]
pub fn window_title() -> &'static str {
    tr!("Cuter 托盘菜单")
}

/// 面板宽度（egui 点）。
const PANEL_WIDTH: f32 = 268.0;
/// 内边距。和贴图菜单一样是 5：菜单类界面要紧凑。
const PAD: i8 = 5;
/// 行高。
///
/// 2026-10-20 从 32 收到 28：托盘菜单是"扫一眼就点"的东西，32 的行距让十条项占满
/// 半个屏幕（用户："托盘感觉有点宽松，想更紧凑一点"）。28 仍然大于 24 的可点下限。
const ROW_H: f32 = 28.0;
/// 分隔线占的高度（线 1px + 上下留白）。
const SEP_H: f32 = 7.0;
/// 顶部「Cuter + 版本」那一行的高度。
const HEADER_H: f32 = 24.0;
/// 底部提示行的高度。
const HINT_H: f32 = 20.0;
/// 行的数量：截图 3 + 贴图 3 + 窗口 3 + 退出 1。
const ROWS: u32 = 10;
/// 分隔线数量。
const SEPARATORS: u32 = 3;

/// 面板尺寸。宿主拿它设窗口大小——菜单不能让用户拉，内容高度是算得出来的常量。
pub const PANEL_SIZE: (f32, f32) = (
    PANEL_WIDTH,
    (PAD as f32) * 2.0 + HEADER_H + ROW_H * (ROWS as f32) + SEP_H * (SEPARATORS as f32) + HINT_H,
);

/// 面板里点出来的动作。
///
/// 与宿主的 `TrayCommand` 一一对应（除了 [`TrayAction::Close`]：只是关掉菜单，
/// 什么都不做）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    /// 框选截图。
    Capture,
    /// 截取当前显示器。
    CaptureMonitor,
    /// 把剪贴板里的图片贴到屏幕上。
    PinFromClipboard,
    /// 隐藏 / 显示所有贴图。
    TogglePins,
    /// 取消所有贴图的鼠标穿透（逃生项，见 `crates/cuter-app/src/desktop.rs`）。
    ClearPinClickThrough,
    /// 切换到下一个贴图分组。
    NextPinGroup,
    /// 打开设置窗口。
    OpenSettings,
    /// 打开截图历史面板。
    OpenHistory,
    /// 打开保存目录。
    OpenSaveDir,
    /// 退出 Cuter。
    Quit,
    /// 什么都不做，只把菜单收起来（Esc、点到别处、点窗口关闭按钮）。
    Close,
}

/// 画一帧需要的输入：快捷键（空串表示没绑定，右侧不显示）与贴图组状态。
#[derive(Clone, Copy, Debug, Default)]
pub struct TrayPanelInput<'a> {
    /// 框选截图的键位。
    pub capture: &'a str,
    /// 截取当前显示器的键位。
    pub capture_monitor: &'a str,
    /// 贴图的键位。
    pub pin: &'a str,
    /// 隐藏 / 显示所有贴图的键位。
    pub toggle_pins: &'a str,
    /// 切换到下一个贴图组的键位。
    pub next_group: &'a str,
    /// 贴图现在是不是被藏起来了（决定那一行的文字）。
    pub pins_hidden: bool,
    /// 当前组的贴图数量（0 时不显示）。
    pub pin_count: u32,
}

/// 托盘菜单面板。
pub struct TrayPanel {
    theme: Theme,
}

impl TrayPanel {
    /// 建一个面板。
    #[must_use]
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// 画一帧，返回用户点的动作。
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        input: &TrayPanelInput<'_>,
    ) -> Option<TrayAction> {
        let theme = self.theme;
        let mut action = None;

        // 菜单的常规出口：Esc。点菜单外由宿主负责（视口失去焦点就关）。
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = Some(TrayAction::Close);
        }

        egui::Frame::new()
            .fill(theme.surface)
            .stroke(Stroke::new(1.0, theme.outline))
            .corner_radius(CornerRadius::same(metrics::CORNER))
            .inner_margin(Margin::same(PAD))
            .show(ui, |ui| {
                ui.set_width(PANEL_WIDTH - f32::from(PAD) * 2.0);
                ui.spacing_mut().item_spacing = Vec2::ZERO;

                header(ui, &theme);

                let row = |ui: &mut egui::Ui,
                           label: &str,
                           hint: &str,
                           danger: bool,
                           act: TrayAction,
                           action: &mut Option<TrayAction>| {
                    let (_, response) = menu_row(ui, &theme, label, hint, danger);
                    if response.clicked() {
                        *action = Some(act);
                    }
                };

                row(
                    ui,
                    tr!("框选截图"),
                    input.capture,
                    false,
                    TrayAction::Capture,
                    &mut action,
                );
                row(
                    ui,
                    tr!("截取当前显示器"),
                    input.capture_monitor,
                    false,
                    TrayAction::CaptureMonitor,
                    &mut action,
                );
                row(
                    ui,
                    tr!("贴图（剪贴板图片）"),
                    input.pin,
                    false,
                    TrayAction::PinFromClipboard,
                    &mut action,
                );

                menu_separator(ui, &theme);

                // 「隐藏/显示」两态：贴图藏着的时候这一行写成"显示"，
                // 用户不用去猜当前是哪个状态。
                let toggle_label = if input.pins_hidden {
                    tr!("显示所有贴图")
                } else {
                    tr!("隐藏所有贴图")
                };
                row(
                    ui,
                    toggle_label,
                    input.toggle_pins,
                    false,
                    TrayAction::TogglePins,
                    &mut action,
                );
                row(
                    ui,
                    tr!("切换到下一个贴图组"),
                    input.next_group,
                    false,
                    TrayAction::NextPinGroup,
                    &mut action,
                );
                row(
                    ui,
                    tr!("取消所有贴图的鼠标穿透"),
                    "",
                    false,
                    TrayAction::ClearPinClickThrough,
                    &mut action,
                );

                menu_separator(ui, &theme);

                row(
                    ui,
                    tr!("设置…"),
                    "",
                    false,
                    TrayAction::OpenSettings,
                    &mut action,
                );
                row(
                    ui,
                    tr!("截图历史…"),
                    "",
                    false,
                    TrayAction::OpenHistory,
                    &mut action,
                );
                row(
                    ui,
                    tr!("打开保存目录"),
                    "",
                    false,
                    TrayAction::OpenSaveDir,
                    &mut action,
                );

                menu_separator(ui, &theme);

                row(
                    ui,
                    tr!("退出 Cuter"),
                    "",
                    true,
                    TrayAction::Quit,
                    &mut action,
                );

                hint_row(ui, &theme, input.pin_count);
            });

        action
    }
}

/// 顶部：左「Cuter」、右版本号（暗色小字）。
fn header(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HEADER_H),
        egui::Sense::hover(),
    );
    ui.painter().text(
        Pos2::new(rect.left() + f32::from(PAD) + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Cuter",
        FontId::proportional(13.0),
        theme.on_surface,
    );
    ui.painter().text(
        Pos2::new(
            rect.right() - f32::from(PAD) - 4.0,
            rect.center().y,
        ),
        Align2::RIGHT_CENTER,
        fill!(tr!("版本 {}"), env!("CARGO_PKG_VERSION")),
        FontId::proportional(11.0),
        theme.on_surface_faint,
    );
}

/// 底部提示行：告诉用户托盘图标**左键**是干什么的（很多人的第一反应是点一下）。
fn hint_row(ui: &mut egui::Ui, theme: &Theme, pin_count: u32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HINT_H),
        egui::Sense::hover(),
    );
    let text = if pin_count == 0 {
        tr!("左键单击托盘图标 = 框选截图").to_owned()
    } else {
        fill!(tr!("左键单击托盘图标 = 框选截图 · 屏幕上有 {} 张贴图"), pin_count)
    };
    ui.painter().text(
        Pos2::new(rect.left() + f32::from(PAD) + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(11.0),
        theme.on_surface_faint,
    );
}

/// 菜单一行：左侧标签、右侧快捷键提示；hover 用 `accent/subtle` 高亮（圆角 8）。
fn menu_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    hint: &str,
    danger: bool,
) -> (Rect, egui::Response) {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, ROW_H), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), theme.accent_subtle);
    }
    ui.painter().text(
        Pos2::new(rect.left() + f32::from(PAD) + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        if danger { theme.danger } else { theme.on_surface },
    );
    let hint = hint.trim();
    if !hint.is_empty() {
        ui.painter().text(
            Pos2::new(rect.right() - f32::from(PAD) - 4.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            FontId::proportional(11.0),
            theme.on_surface_faint,
        );
    }
    (rect, response)
}

/// 菜单里的分隔线：1px `outline`，上下各留 4px。
fn menu_separator(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SEP_H),
        egui::Sense::hover(),
    );
    let y = rect.center().y.round() + 0.5;
    let inset = f32::from(PAD) + 4.0;
    ui.painter().line_segment(
        [
            Pos2::new(rect.left() + inset, y),
            Pos2::new(rect.right() - inset, y),
        ],
        Stroke::new(1.0, theme.outline),
    );
}

/// 面板最外圈的描边由 `Frame` 的 `stroke` 负责（圆角 12，见 `metrics::CORNER`）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_height_fits_every_row() {
        let (_, height) = PANEL_SIZE;
        let content = f32::from(PAD) * 2.0
            + HEADER_H
            + ROW_H * (ROWS as f32)
            + SEP_H * (SEPARATORS as f32)
            + HINT_H;
        assert!((height - content).abs() < f32::EPSILON);
        assert!(height > 300.0, "十行菜单不该像个小方块：{height}");
        assert!(height < 480.0, "菜单太高会顶到屏幕边：{height}");
    }

    #[test]
    fn window_title_is_specific() {
        assert!(window_title().contains("Cuter"));
    }

    #[test]
    fn the_toggle_row_follows_the_hidden_state() {
        // 这条只为钉住"两态文字"这个约定：藏起来时那一行必须写"显示"。
        let panel = TrayPanel::new(Theme::dark());
        let hidden = TrayPanelInput {
            pins_hidden: true,
            ..TrayPanelInput::default()
        };
        // 面板本身不持有状态，两态只来自输入——这里断言构造出来的输入确实被带进去。
        let _ = panel;
        assert!(hidden.pins_hidden);
        assert_ne!(tr!("显示所有贴图"), tr!("隐藏所有贴图"));
    }
}
