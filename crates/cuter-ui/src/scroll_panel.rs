//! 滚动截屏的控制面板。
//!
//! 采集与拼接在后台线程上跑（见宿主的 `ScrollSession`），这个面板只做两件事：
//!
//! * 让用户**看到正在拼出什么**——一张实时更新的缩略长图，外加帧数与当前尺寸；
//! * 提供出口：完成 / 暂停 / 取消。
//!
//! ## 版面按设计稿
//!
//! 骨架与 Figma 画板 `40:230`「滚动截屏控制面板 · 设计稿」（导出图
//! `docs/design/mockups/scroll-panel.png`）一致：内边距 16、顶栏是标题 + 三个元信息胶囊
//! （帧数 / 拼接尺寸 / 方向）+ 右侧「自动滚动」与「取消」、中间一块等比缩放的拼接预览
//! （圆角 8）、底部 11px 状态行 + 键盘提示，右下角**唯一**的实心按钮是「完成 (Enter)」。
//!
//! ## 为什么预览是缩略图而不是原图
//!
//! 长图会长到几千甚至上万行，而 GL 的纹理边长上限通常是 16384——直接把原图传上去迟早
//! 会失败，而且每拼一帧就重传一次几百 MB 的纹理也不现实。所以宿主只按固定间隔下发一张
//! **缩略图**（`preview_generation` 自增表示有新内容），面板据此重建纹理。
//! 原图始终留在宿主那边，导出时才取。

use crate::{fill, tr};

use egui::{Align2, Color32, CornerRadius, Margin, RichText, Stroke, StrokeKind, Vec2};

use crate::annotate::frame_to_color_image;
use crate::theme::{metrics, Theme};
use cuter_core::geom::Size;

/// 面板上的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAction {
    /// 结束采集并把长图交付出去。
    Finish,
    /// 暂停 / 继续采集。
    TogglePause,
    /// 放弃这次滚动截屏。
    Cancel,
    /// 开关自动滚动（程序自己合成滚轮，用户不用手滚）。
    ToggleAuto,
}

/// 面板每帧需要的输入（由宿主从会话状态里取好再传进来，避免面板持有锁）。
pub struct ScrollPanelInput<'a> {
    pub size: Size,
    pub frames: u32,
    /// 当前判定的滚动方向（`纵向` / `横向`）：用户滚哪边，程序就往哪边拼。
    pub axis: &'a str,
    pub status: &'a str,
    pub paused: bool,
    /// 自动滚动是否开着（面板据此显示状态）。
    pub auto: bool,
    pub max_height: u32,
    /// 有新预览时自增；面板据此决定是否重建纹理。
    pub preview_generation: u64,
    /// 缩略长图；`None` 表示还没有可显示的内容。
    pub preview: Option<&'a cuter_core::frame::Frame>,
}

/// 滚动截屏控制面板。
pub struct ScrollPanel {
    theme: Theme,
    texture: Option<egui::TextureHandle>,
    /// 已经上传过的预览代号，避免每帧重传纹理。
    uploaded_generation: u64,
}

/// 底板内边距（设计稿：16）。
const PAD: i8 = 16;
/// 预览框与底部状态区的圆角（设计稿：预览 8）。
const CORNER_BOX: u8 = 8;

impl ScrollPanel {
    #[must_use]
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            texture: None,
            uploaded_generation: u64::MAX,
        }
    }

    /// 每帧绘制。返回本帧发生的动作。
    pub fn ui(&mut self, ui: &mut egui::Ui, input: &ScrollPanelInput<'_>) -> Option<ScrollAction> {
        let mut action = None;
        let theme = self.theme;

        // 键盘出口：这个窗口是「采集进行中」的唯一出口，用户不该被迫去精准点按钮。
        // 面板里没有任何文本输入框，所以这几个键不会和输入冲突。
        let (enter, escape, space, auto_key) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::A),
            )
        });
        if escape {
            action = Some(ScrollAction::Cancel);
        } else if auto_key {
            action = Some(ScrollAction::ToggleAuto);
        } else if enter || space {
            action = Some(ScrollAction::Finish);
        }

        // 底板：1px 描边画在窗口边缘，内容整体缩进 PAD
        let outer = ui.max_rect();
        egui::Frame::new()
            .inner_margin(Margin::same(PAD))
            .show(ui, |ui| {
                // ---- 顶栏：右侧出口先占位，剩下的宽度留给标题与元信息 ----
                // 反过来写（先标题后 right_to_left）会让右侧那组拿到「已被标题吃掉」的
                // 剩余矩形，字一宽就压到胶囊上——真机截图里「自动滚动：开 (A)」就是骑在
                // 「竖滚」胶囊上的。
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if outline_button(ui, &theme, tr!("取消")) {
                            action = Some(ScrollAction::Cancel);
                        }
                        // 自动滚动是个「文字 + 快捷键」的开关，不是按钮：设计稿里它不抢焦点
                        let auto_text = if input.auto {
                            tr!("自动滚动：开 (A)")
                        } else {
                            tr!("自动滚动：关 (A)")
                        };
                        let auto_color = if input.auto {
                            theme.accent
                        } else {
                            theme.on_surface_dim
                        };
                        if text_button(ui, &theme, auto_text, auto_color) {
                            action = Some(ScrollAction::ToggleAuto);
                        }

                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(tr!("滚动截屏"))
                                    .size(metrics::TEXT_TITLE_2)
                                    .strong()
                                    .color(theme.on_surface),
                            );
                            ui.add_space(metrics::SPACE_2);
                            chip(ui, &theme, &fill!(tr!("{} 帧"), input.frames));
                            if input.size.w > 0 && input.size.h > 0 {
                                chip(ui, &theme, &fill!(tr!("{} × {}"), input.size.w, input.size.h));
                            }
                            chip(ui, &theme, axis_chip(input.axis));
                        });
                    });
                });

                ui.add_space(metrics::SPACE_3);

                // ---- 中间：拼接预览（没有内容时退化成提示文案） ----
                let has_preview = input.preview.is_some();
                // 底部那块要几行：状态行 + 快捷键行 + 按钮（+ 进度条），给足余量再让预览吃剩下的。
                // 余量给少了的表现很隐蔽：状态行与「完成」按钮会被挤到窗口外面半行。
                let bottom_h = if input.max_height > 0 { 92.0 } else { 72.0 };
                let hints_h = if has_preview { 0.0 } else { 48.0 };
                let box_h = (ui.available_height() - bottom_h - hints_h).max(72.0);
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), box_h),
                    egui::Sense::hover(),
                );
                self.preview_box(ui, rect, input);
                if !has_preview {
                    ui.add_space(metrics::SPACE_2);
                    ui.label(
                        RichText::new(tr!("请在要截取的窗口里正常滚动，我会自动把新内容拼到下面"))
                            .size(metrics::TEXT_LABEL)
                            .color(theme.on_surface_dim),
                    );
                    ui.label(
                        RichText::new(tr!("长度到上限时会提示「新的滚动内容不会被再追加」"))
                            .size(metrics::TEXT_CAPTION)
                            .color(theme.on_surface_faint),
                    );
                }

                // ---- 长截图进度：已拼高度 / 长度上限 ----
                // 面板上的「帧数 / 拼接尺寸」是快照，看不出「离上限还有多远」——这条 4px 的
                // 轨道就是那个比例。上限没设置（0）时不画，免得空占一行。
                if input.max_height > 0 {
                    ui.add_space(metrics::SPACE_2);
                    progress_bar(ui, &theme, progress_ratio(input.size.h, input.max_height));
                }

                // ---- 底部：状态行 + 键盘提示（左）与唯一的实心按钮（右） ----
                // 不能用 Layout::bottom_up 包一层：它在「当前光标」处再自下而上排，
                // 而这里光标已经在窗口底部，结果是整块被推到窗口外面（真机截图里
                // 「完成 (Enter)」只剩半颗）。普通自上而下排，高度靠上面的预留保证。
                ui.add_space(metrics::SPACE_2);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(status_text(input))
                                .size(metrics::TEXT_CAPTION)
                                .color(status_color(input, &theme)),
                        );
                        ui.label(
                            RichText::new(tr!("回车完成 · 空格暂停/继续 · Esc 取消"))
                                .size(metrics::TEXT_CAPTION)
                                .color(theme.on_surface_faint),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, &theme, tr!("完成 (Enter)")) {
                            action = Some(ScrollAction::Finish);
                        }
                    });
                });
            });

        // 底板描边（画在最外层，压住内容边缘）
        ui.painter().rect_stroke(
            outer.shrink(0.5),
            CornerRadius::ZERO,
            Stroke::new(1.0, theme.outline),
            StrokeKind::Inside,
        );

        action
    }

    /// 中间的预览框：底色 + 等比缩放的缩略图 + 上限提示。
    fn preview_box(&mut self, ui: &mut egui::Ui, rect: egui::Rect, input: &ScrollPanelInput<'_>) {
        let theme = self.theme;
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            CornerRadius::same(CORNER_BOX),
            theme.surface_sunken,
            Stroke::new(1.0, theme.outline),
            StrokeKind::Inside,
        );

        let Some(preview) = input.preview else {
            return;
        };
        if preview.width() == 0 || preview.height() == 0 {
            return;
        }

        // 纹理只在预览代号变化时重传
        if input.preview_generation != self.uploaded_generation {
            let image = frame_to_color_image(preview);
            match &mut self.texture {
                Some(handle) => handle.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "cuter-scroll-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            self.uploaded_generation = input.preview_generation;
        }

        // 缩放：铺满宽度、高度按比例；两者取更紧的那个约束
        let inner = rect.shrink(4.0);
        let aspect = preview.height() as f32 / preview.width() as f32;
        if inner.width() <= 8.0 || inner.height() <= 8.0 {
            return;
        }
        let width = inner.width().min(inner.height() / aspect.max(0.001));
        let view = Vec2::new(width, width * aspect);
        let target = egui::Rect::from_center_size(inner.center(), view);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                target,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.rect_stroke(
            target,
            CornerRadius::same(4),
            Stroke::new(1.0, theme.outline),
            StrokeKind::Inside,
        );
        // 标出长度上限的位置，让用户知道什么时候会被截断
        if input.max_height > 0 && input.size.h >= input.max_height {
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - 10.0),
                Align2::CENTER_BOTTOM,
                tr!("已达长度上限，新的滚动内容不会再被追加"),
                egui::FontId::proportional(metrics::TEXT_CAPTION),
                theme.danger,
            );
        }
    }
}

/// 控制面板窗口标题。
/// 窗口标题。跟着界面语言走，所以是个函数而不是常量。
#[must_use]
pub fn window_title() -> &'static str {
    tr!("Cuter 滚动截屏")
}

/// 底栏那行状态字：优先说「正在发生什么」，其次才回落到宿主给的会话状态。
fn status_text(input: &ScrollPanelInput<'_>) -> String {
    if input.paused {
        return tr!("已暂停 · 空格继续").to_owned();
    }
    if input.max_height > 0 && input.size.h >= input.max_height {
        return tr!("已达长度上限，新的滚动内容不会被再追加").to_owned();
    }
    if input.auto {
        return tr!("正在自动滚动…按 A 停下").to_owned();
    }
    if input.status.is_empty() || input.frames == 0 {
        return tr!("等待第一帧…").to_owned();
    }
    input.status.to_owned()
}

/// 状态字的颜色：只有「暂停」和「到上限」值得用彩色，其余走暗色不抢主角。
fn status_color(input: &ScrollPanelInput<'_>, theme: &Theme) -> Color32 {
    if input.max_height > 0 && input.size.h >= input.max_height {
        theme.danger
    } else if input.paused {
        theme.accent
    } else {
        theme.on_surface_dim
    }
}

/// 方向胶囊的短名：面板上只有一格宽，写成「竖滚 / 横滚」。
fn axis_chip(axis: &str) -> &str {
    match axis {
        "纵向" => tr!("竖滚"),
        "横向" => tr!("横滚"),
        other => other,
    }
}

/// 长截图进度：已拼高度相对长度上限的比例。上限为 0 表示没设上限，没有进度可谈。
fn progress_ratio(height: u32, max_height: u32) -> f32 {
    if max_height == 0 {
        return 0.0;
    }
    (height as f32 / max_height as f32).clamp(0.0, 1.0)
}

/// 一条 4px 高的进度轨：轨道用 `outline`，已拼的部分用 `accent`。
/// 比例很小时也留一个 4px 的小节，否则「已经开始了」这件事在屏幕上根本看不见。
fn progress_bar(ui: &mut egui::Ui, theme: &Theme, ratio: f32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 4.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), theme.outline);
    if ratio > 0.0 {
        let width = (rect.width() * ratio).max(4.0);
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(width, rect.height())),
            CornerRadius::same(2),
            theme.accent,
        );
    }
}

/// 顶栏上的元信息胶囊（和 OCR 面板、历史面板、设置页同一套观感）。
fn chip(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    egui::Frame::new()
        .fill(theme.label_bg)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(metrics::TEXT_LABEL)
                    .color(theme.on_surface_dim),
            );
        });
}

/// 描边按钮（次要动作：「取消」）。
fn outline_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new((text.chars().count() as f32 * 13.0 + 22.0).max(56.0), 28.0),
        egui::Sense::click(),
    );
    let fill = if response.hovered() {
        theme.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(metrics::CORNER_BUTTON),
        fill,
        Stroke::new(1.0, theme.outline),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(metrics::TEXT_LABEL),
        theme.on_surface,
    );
    response.clicked()
}

/// 实心强调按钮：这一屏里只应该有「完成」一个。
fn primary_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new((text.chars().count() as f32 * 12.0 + 26.0).max(72.0), 30.0),
        egui::Sense::click(),
    );
    let fill = if response.hovered() {
        theme.accent_hover
    } else {
        theme.accent
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(metrics::CORNER_BUTTON),
        fill,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(metrics::TEXT_LABEL),
        Color32::WHITE,
    );
    response.clicked()
}

/// 只留文字的开关（「自动滚动：开 (A)」）：hover 时给一层很淡的底。
fn text_button(ui: &mut egui::Ui, theme: &Theme, text: &str, color: Color32) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(text.chars().count() as f32 * 11.5 + 12.0, 24.0),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect(
            rect,
            CornerRadius::same(metrics::CORNER_INPUT),
            theme.surface_hover,
            Stroke::NONE,
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(metrics::TEXT_LABEL),
        color,
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_starts_without_a_texture() {
        let panel = ScrollPanel::new(Theme::dark());
        assert!(panel.texture.is_none());
        assert_eq!(panel.uploaded_generation, u64::MAX, "首帧必须触发一次上传");
    }

    #[test]
    fn window_title_is_specific() {
        // 宿主靠标题找窗口、探针也靠它做断言，别改成一个含糊的名字
        assert!(window_title().contains("Cuter"));
    }

    #[test]
    fn the_axis_chip_is_short() {
        // 顶栏只有一格宽，方向得用短名（宿主传进来的仍是「纵向 / 横向」）
        assert_eq!(axis_chip("纵向"), "竖滚");
        assert_eq!(axis_chip("横向"), "横滚");
        assert_eq!(axis_chip("别的"), "别的", "不认识的方向原样显示");
    }

    /// 在一个真实的 egui 上下文里跑一帧，注入按键，看面板吐出什么动作。
    fn drive(panel: &mut ScrollPanel, key: Option<egui::Key>) -> Option<ScrollAction> {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        if let Some(key) = key {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            });
        }
        let mut action = None;
        let preview = cuter_core::frame::Frame::new(Size::new(4, 40));
        let panel_input = ScrollPanelInput {
            axis: "纵向",
            size: Size::new(4, 40),
            frames: 3,
            status: tr!("已拼接"),
            paused: false,
            auto: false,
            max_height: 1000,
            preview_generation: 1,
            preview: Some(&preview),
        };
        // 纹理增量没人消费（测试里没有渲染后端），显式丢弃，否则 epaint 会在 drop 时 panic
        ctx.run_ui(input, |ui| {
            action = panel.ui(ui, &panel_input);
        })
        .drop_without_applying_deltas();
        action
    }

    #[test]
    fn enter_finishes_and_escape_cancels() {
        // 采集期间唯一的出口就是这个窗口，键盘必须能用
        let mut panel = ScrollPanel::new(Theme::dark());
        assert_eq!(
            drive(&mut panel, Some(egui::Key::Enter)),
            Some(ScrollAction::Finish)
        );
        assert_eq!(
            drive(&mut panel, Some(egui::Key::Escape)),
            Some(ScrollAction::Cancel)
        );
        assert_eq!(
            drive(&mut panel, Some(egui::Key::Space)),
            Some(ScrollAction::Finish)
        );
        assert_eq!(
            drive(&mut panel, Some(egui::Key::A)),
            Some(ScrollAction::ToggleAuto)
        );
    }

    #[test]
    fn no_key_means_no_action() {
        let mut panel = ScrollPanel::new(Theme::dark());
        assert_eq!(drive(&mut panel, None), None);
    }

    /// 底栏状态字：把「正在发生什么」排在宿主状态之前。
    fn status_of(paused: bool, auto: bool, frames: u32, size_h: u32, max: u32) -> String {
        let input = ScrollPanelInput {
            size: Size::new(4, size_h),
            frames,
            axis: "纵向",
            status: "已拼接",
            paused,
            auto,
            max_height: max,
            preview_generation: 0,
            preview: None,
        };
        status_text(&input)
    }

    #[test]
    fn the_status_line_tells_what_is_happening() {
        assert_eq!(status_of(true, false, 3, 40, 1000), tr!("已暂停 · 空格继续"));
        assert_eq!(
            status_of(false, true, 3, 40, 1000),
            tr!("正在自动滚动…按 A 停下")
        );
        assert_eq!(
            status_of(false, false, 3, 1000, 1000),
            tr!("已达长度上限，新的滚动内容不会被再追加")
        );
        assert_eq!(status_of(false, false, 0, 0, 1000), tr!("等待第一帧…"));
        assert_eq!(status_of(false, false, 3, 40, 1000), "已拼接");
    }

    #[test]
    fn the_status_color_does_not_shout() {
        let theme = Theme::dark();
        let input = ScrollPanelInput {
            size: Size::new(4, 40),
            frames: 3,
            axis: "纵向",
            status: "已拼接",
            paused: false,
            auto: false,
            max_height: 1000,
            preview_generation: 0,
            preview: None,
        };
        assert_eq!(status_color(&input, &theme), theme.on_surface_dim);
        let paused = ScrollPanelInput { paused: true, ..input };
        assert_eq!(status_color(&paused, &theme), theme.accent);
        let capped = ScrollPanelInput {
            size: Size::new(4, 1000),
            max_height: 1000,
            ..paused
        };
        assert_eq!(status_color(&capped, &theme), theme.danger);
    }

    #[test]
    fn the_progress_bar_reads_the_length_cap() {
        // 没设上限 = 没有进度可谈
        assert_eq!(progress_ratio(400, 0), 0.0);
        assert_eq!(progress_ratio(0, 8000), 0.0);
        assert_eq!(progress_ratio(2000, 8000), 0.25);
        // 到顶与超过都夹在 1.0（上限是「不再追加」的判据，不是硬约束）
        assert_eq!(progress_ratio(8000, 8000), 1.0);
        assert_eq!(progress_ratio(9000, 8000), 1.0);
    }
}
