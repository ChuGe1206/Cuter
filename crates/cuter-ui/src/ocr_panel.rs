//! OCR 识别结果面板。
//!
//! 布局刻意做成 **左图右文**：左边是最后一次框选的图，上面叠着识别出的文本块；
//! 右边是可编辑的识别文本。之所以要「可编辑」，是因为 OCR 必然会有错字——
//! 用户校对完再复制，比复制出来再去别处改要顺手得多。
//!
//! 交互约定：
//!
//! * 鼠标移到图上某个文本块 → 高亮该块；单击 → 复制**这一行**（取词场景最常用）；
//! * 右侧文本框可随意编辑，工具栏的「复制」复制的是**编辑后**的内容；
//! * 「复制并关闭」是最常用的出口，因此给它一个明显的位置。
//!
//! 尚未做：把光标在右侧文本里的位置反向映射回左侧的块（需要把字符偏移与
//! `TextBlock` 对齐，同类工具有这个效果）。这一版只做了「图 → 文本」方向的联动。

use crate::{fill, tr};

use egui::{Color32, CornerRadius, Sense, Stroke, StrokeKind, Vec2};

use crate::annotate::frame_to_color_image;
use crate::theme::Theme;
use cuter_core::frame::Frame;
use cuter_core::ocr::OcrResult;

/// 面板上的动作。
#[derive(Clone, Debug, PartialEq)]
pub enum OcrAction {
    /// 复制编辑后的全部文本，面板保持打开。
    Copy,
    /// 复制并关闭面板。
    CopyAndClose,
    /// 直接关闭。
    Close,
    /// 复制某一个文本块（点击左侧图上的框，或右侧"逐行对照"里的一行）。
    CopyBlock(String),
    /// 用同一张图重新识别一次（设计稿底栏的「重新识别」）。
    Retry,
}

/// 右侧显示什么：可编辑全文 / 逐行对照。
///
/// **为什么要有"逐行对照"这个模式**：文字↔位置的双向联动只在"行号与文本块一一对应"
/// 时才成立。全文框是可编辑的（校对用），用户改两个字之后行号就对不上了，
/// 硬做联动只会指错地方。所以联动放在只读的列表模式里，两个模式各管一件事。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ViewMode {
    /// 可编辑全文（校对、整段复制）。
    #[default]
    Text,
    /// 逐行列表：鼠标在哪一行，图上就高亮哪个块，反过来也一样。
    Lines,
}

/// 识别结果面板。
pub struct OcrPanel {
    frame: Frame,
    result: OcrResult,
    /// 可编辑的全文，初始值来自 OCR。
    text: String,
    /// 引擎实际使用的语言，显示在标题栏。
    language: String,
    texture: Option<egui::TextureHandle>,
    theme: Theme,
    /// 鼠标当前悬停的文本块序号（图上命中或列表行命中，见 [`Self::row_hover`]）。
    hovered: Option<usize>,
    /// 逐行列表里鼠标悬停的行。和 `hovered` 分开存是因为两边**绘制顺序不同**：
    /// 左栏先画、右栏后画，共用一个字段就会互相覆盖（0.1.48 之前正是如此，
    /// 每帧画框时读到的永远是刚被清掉的 `None`）。
    row_hover: Option<usize>,
    /// 被选中的文本块（点过之后保持高亮，另一侧会滚动到它）。
    selected: Option<usize>,
    /// 右侧视图模式。
    mode: ViewMode,
    /// 需要把选中的那一行滚进视野（置一次，用完清掉）。
    scroll_to_selected: bool,
    status: Option<String>,
    status_frames: u32,
}

impl OcrPanel {
    /// 构造结果面板。
    ///
    /// `paragraph` 来自 `config.ocr.paragraph`：按标点断行还是按识别块一行一块。
    /// 它必须由调用方传进来——面板自己写死 `true` 会让设置界面里那个
    /// 「段落模式」开关变成一枚假开关。
    #[must_use]
    pub fn new(
        frame: Frame,
        result: OcrResult,
        language: impl Into<String>,
        paragraph: bool,
        theme: Theme,
    ) -> Self {
        let text = result.plain_text(paragraph);
        Self {
            frame,
            result,
            text,
            language: language.into(),
            texture: None,
            theme,
            hovered: None,
            row_hover: None,
            selected: None,
            mode: ViewMode::default(),
            scroll_to_selected: false,
            status: None,
            status_frames: 0,
        }
    }

    /// 当前（可能已被用户编辑过的）全文。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 这一版识别用的那张图（「重新识别」要拿它再跑一遍）。
    #[must_use]
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// 识别到的文本块数量。
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.result.blocks.len()
    }

    /// 给面板一个一次性提示（例如「已复制」）。
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
        self.status_frames = 150;
    }

    fn texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.texture.is_none() {
            let image = frame_to_color_image(&self.frame);
            self.texture = Some(ctx.load_texture(
                "cuter-ocr-preview",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        self.texture.as_ref().expect("刚刚写入过").id()
    }

    /// 每帧绘制面板。
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<OcrAction> {
        // Esc 关闭：设计稿的标题栏上没有关闭按钮，靠窗口的 ✗ 或这个键
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            return Some(OcrAction::Close);
        }
        // Ctrl+Enter 复制并关闭：设计稿底栏只有「重新识别 / 复制全部」两个按钮，
        // 「复制完就走」这个常用出口改成快捷键，不占位置。
        if ui.ctx().input(|i| {
            i.key_pressed(egui::Key::Enter) && i.modifiers.command
        }) {
            return Some(OcrAction::CopyAndClose);
        }
        // `L` 键切换视图：探针与键盘用户都不用去点那个小按钮
        if ui.ctx().input(|i| i.key_pressed(egui::Key::L)) {
            self.mode = match self.mode {
                ViewMode::Text => ViewMode::Lines,
                ViewMode::Lines => ViewMode::Text,
            };
        }
        if self.status_frames > 0 {
            self.status_frames -= 1;
            if self.status_frames == 0 {
                self.status = None;
            }
        }

        let mut action = None;

        // ---- 标题栏：标题 + 元信息胶囊，右侧是视图切换 ----
        ui.horizontal(|ui| {
            ui.heading(tr!("识别结果"));
            ui.add_space(6.0);
            chip_label(ui, &self.theme, &self.language);
            chip_label(ui, &self.theme, &fill!(tr!("{} 行"), self.result.blocks.len()));
            chip_label(ui, &self.theme, &format!("{} ms", self.result.elapsed_ms));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(status) = &self.status {
                    ui.label(
                        egui::RichText::new(status)
                            .color(self.theme.accent)
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                }
                let lines = self.mode == ViewMode::Lines;
                if mode_button(ui, &self.theme, tr!("逐行对照 (L)"), lines).clicked() {
                    self.mode = ViewMode::Lines;
                }
                if mode_button(ui, &self.theme, tr!("编辑全文"), !lines).clicked() {
                    self.mode = ViewMode::Text;
                }
            });
        });
        ui.separator();

        if self.result.blocks.is_empty() {
            // 空结果也把提示放在「正文」那一格里，底栏才会贴在面板底部
            let available = ui.available_size();
            let body_height = (available.y - 80.0).max(80.0);
            ui.allocate_ui_with_layout(
                Vec2::new(available.x, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(body_height);
                    ui.label(
                        egui::RichText::new(tr!("没有识别到文字。可以试试：放大框选范围、避免半透明背景、或换成更清晰的字体。"))
                            .color(self.theme.on_surface_dim),
                    );
                },
            );
            footer(
                ui,
                &self.theme,
                self.result.blocks.len(),
                self.result.elapsed_ms,
                &mut action,
            );
            return action;
        }

        // ---- 左图右文 ----
        // 底栏（行数 + 按钮）要占一行：先把它的高度留出来，
        // 否则正文会把底栏顶到面板外面去（第一版就是这样，抓图才发现）。
        let available = ui.available_size();
        let body_height = (available.y - 80.0).max(80.0);
        let left_width = (available.x * 0.46).clamp(200.0, 720.0);

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_width, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(body_height);
                    if let Some(clicked) = self.preview(ui, left_width) {
                        action = Some(clicked);
                    }
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(body_height);
                    ui.label(
                        egui::RichText::new(tr!("识别文本"))
                            .color(self.theme.on_surface_dim)
                            .size(12.0),
                    );
                    match self.mode {
                        ViewMode::Text => {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.text)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(24)
                                            .font(egui::TextStyle::Monospace),
                                    );
                                });
                        }
                        ViewMode::Lines => {
                            if let Some(a) = self.line_list(ui) {
                                action = Some(a);
                            }
                        }
                    }
                    // 设计稿：列表下方的提示框
                    egui::Frame::new()
                        .fill(self.theme.surface_sunken)
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            for line in [
                                tr!("按 Ctrl+C 复制选中行 · H 悬停高亮对应框"),
                                tr!("Ctrl+Enter 复制并关闭 · Esc 关闭"),
                            ] {
                                ui.label(
                                    egui::RichText::new(line)
                                        .size(11.0)
                                        .color(self.theme.on_surface_dim),
                                );
                            }
                        });
                },
            );
        });

        // ---- 底栏：左侧行数与耗时，右侧「重新识别 / 复制全部」----
        footer(
            ui,
            &self.theme,
            self.result.blocks.len(),
            self.result.elapsed_ms,
            &mut action,
        );

        action
    }

    /// 逐行列表：鼠标在哪一行，左边图上就高亮哪个块——**双向联动的另一半**。
    ///
    /// 列表是只读的（不是那个可编辑的全文框），所以"行号 ↔ 文本块"的对应关系是稳的。
    fn line_list(&mut self, ui: &mut egui::Ui) -> Option<OcrAction> {
        let mut action = None;
        // 本帧重新收集：指针离开所有行时这一份必须是空的，否则图上那块会一直亮着。
        // （上一帧的值留在 `row_hover` 里，正好给左栏下一帧画框用。）
        let previous = self.row_hover;
        self.row_hover = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, block) in self.result.blocks.iter().enumerate() {
                    // 图上命中的那一块，它对应的行也跟着亮——联动的另一半方向。
                    let hovered =
                        self.hovered == Some(index) || previous == Some(index);
                    let chosen = self.selected == Some(index);
                    let fill = if chosen {
                        self.theme.accent_subtle
                    } else if hovered {
                        self.theme.surface_hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    let stroke = if chosen {
                        Stroke::new(1.0, self.theme.accent)
                    } else {
                        Stroke::NONE
                    };
                    let mut copied = false;
                    let row = egui::Frame::new()
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                badge(ui, &self.theme, index + 1);
                                ui.label(
                                    egui::RichText::new(block.text.trim())
                                        .size(12.0)
                                        .color(self.theme.on_surface),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button(tr!("复制")).clicked() {
                                            copied = true;
                                        }
                                    },
                                );
                            });
                        })
                        .response;
                    // 选中的那一行滚进视野（从图上点过来时尤其重要：
                    // 识别结果几十行，不滚过去用户根本看不到对应关系）
                    if chosen && self.scroll_to_selected {
                        ui.scroll_to_rect(row.rect, Some(egui::Align::Center));
                        self.scroll_to_selected = false;
                    }
                    let row = row.interact(Sense::click());
                    if row.hovered() {
                        self.row_hover = Some(index);
                    }
                    if copied || row.clicked() {
                        self.selected = Some(index);
                        action = Some(OcrAction::CopyBlock(block.text.clone()));
                    }
                }
            });
        action
    }

    /// 左侧预览图 + 文本块叠框。返回点击某个块时应当执行的动作。
    fn preview(&mut self, ui: &mut egui::Ui, width: f32) -> Option<OcrAction> {
        let size = self.frame.size();
        if size.is_empty() {
            return None;
        }
        let aspect = size.h as f32 / size.w as f32;
        // 小图允许放大，方便看清；但不无限放大
        let scale = (width / size.w as f32).min(4.0);
        let view = Vec2::new(size.w as f32 * scale, size.h as f32 * scale);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, view.y.min(aspect * width)), Sense::click());

        let texture = self.texture(ui.ctx());
        let painter = ui.painter_at(rect);
        painter.image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        // 鼠标位置（换算回图片像素），用于命中测试
        let pointer = response.hover_pos();
        let to_image = |p: egui::Pos2| {
            egui::pos2(
                (p.x - rect.left()) / scale,
                (p.y - rect.top()) / scale,
            )
        };

        // 命中测试交给 `OcrResult::block_at`（纯几何、有单测）：那里在重叠时给最小的块，
        // 用户指着一小段时想要的就是那一小段。悬停行另存 `row_hover`，两边各写各的，
        // 绘制时取并集——这样"指针在列表上"也不会把图上那份命中抹掉（0.1.49 修，
        // 之前共用一个字段，左栏先画、每帧都先把右栏刚设的值覆盖成 None）。
        self.hovered = pointer.and_then(|p| {
            let local = to_image(p);
            self.result.block_at(cuter_core::geom::Point::new(
                local.x.round().max(0.0) as i32,
                local.y.round().max(0.0) as i32,
            ))
        });
        for (index, block) in self.result.blocks.iter().enumerate() {
            let hit = self.hovered == Some(index) || self.row_hover == Some(index);
            let chosen = self.selected == Some(index);
            let fill =
                Color32::from_rgba_unmultiplied(0, 145, 255, if chosen { 85 } else { 60 });
            let stroke = Stroke::new(if hit || chosen { 2.0 } else { 1.0 }, self.theme.accent);

            // 框按**四边形**画：系统引擎给的是轴对齐矩形（画圆角矩形更好看），在线引擎
            // 可能给真正的旋转/竖排四边形——那时必须画多边形，否则斜框外面那几个"空白角"
            // 也会被涂上高亮，看着像框错了字。
            let to_screen = |p: cuter_core::geom::Point| {
                egui::pos2(
                    rect.min.x + p.x as f32 * scale,
                    rect.min.y + p.y as f32 * scale,
                )
            };
            let badge_anchor = if block.is_axis_aligned() {
                let box_rect = egui::Rect::from_min_size(
                    rect.min
                        + Vec2::new(block.bounds.x as f32 * scale, block.bounds.y as f32 * scale),
                    Vec2::new(
                        block.bounds.w as f32 * scale,
                        block.bounds.h as f32 * scale,
                    ),
                );
                if hit || chosen {
                    painter.rect_filled(box_rect, CornerRadius::same(2), fill);
                }
                painter.rect_stroke(
                    box_rect,
                    CornerRadius::same(2),
                    stroke,
                    StrokeKind::Outside,
                );
                box_rect.left_top()
            } else {
                let corners: Vec<egui::Pos2> = block.quad.iter().map(|p| to_screen(*p)).collect();
                if hit || chosen {
                    painter.add(egui::Shape::convex_polygon(
                        corners.clone(),
                        fill,
                        Stroke::NONE,
                    ));
                }
                painter.add(egui::Shape::closed_line(corners, stroke));
                to_screen(block.top_left_corner())
            };

            // 设计稿：每个框左上角带编号，和右侧行号一一对应（斜框挂在最上的那个角）
            let badge_rect = egui::Rect::from_min_size(
                egui::pos2(badge_anchor.x, (badge_anchor.y - 14.0).max(rect.top())),
                Vec2::new(16.0, 14.0),
            );
            painter.rect_filled(badge_rect, CornerRadius::same(3), self.theme.accent);
            painter.text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                (index + 1).to_string(),
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );
        }

        // 悬停时在图上显示该行文本，省得来回看右侧
        if let (Some(index), Some(p)) = (self.hovered, pointer)
            && let Some(block) = self.result.blocks.get(index)
        {
            let label = egui::RichText::new(&block.text)
                .color(self.theme.label_fg)
                .size(12.0);
            let galley = ui.painter().layout_no_wrap(
                block.text.clone(),
                egui::FontId::proportional(12.0),
                self.theme.label_fg,
            );
            let padding = Vec2::new(6.0, 3.0);
            let chip = egui::Rect::from_min_size(
                egui::pos2(p.x + 14.0, p.y + 14.0),
                galley.size() + padding * 2.0,
            );
            let chip = chip.intersect(ui.ctx().viewport_rect());
            ui.painter()
                .rect_filled(chip, CornerRadius::same(4), self.theme.label_bg);
            ui.painter().galley(chip.min + padding, galley, self.theme.label_fg);
            let _ = label;
        }

        if response.clicked()
            && let Some(index) = self.hovered
        {
            // 选中并让右侧列表滚到这一行（双向联动的那一半：图 → 文）
            self.selected = Some(index);
            self.scroll_to_selected = true;
            if let Some(block) = self.result.blocks.get(index) {
                return Some(OcrAction::CopyBlock(block.text.clone()));
            }
        }

        ui.label(
            egui::RichText::new(tr!("单击图中的框可复制该行文字；切到「逐行对照」可以直接看哪行对哪块"))
                .color(self.theme.on_surface_dim)
                .size(11.0),
        );
        None
    }
}

/// 标题栏里的元信息胶囊（语言 / 行数 / 耗时）。
fn chip_label(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(11.0),
        theme.on_surface_dim,
    );
    let padding = Vec2::new(7.0, 3.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), theme.label_bg);
    painter.galley(rect.min + padding, galley, theme.on_surface_dim);
}

/// 视图切换按钮：选中的那个用 accent 底 + accent 文字（设计稿的「编辑全文 / 逐行对照」）。
fn mode_button(ui: &mut egui::Ui, theme: &Theme, text: &str, active: bool) -> egui::Response {
    let (fill, color) = if active {
        (theme.accent_subtle, theme.accent)
    } else {
        (Color32::TRANSPARENT, theme.on_surface_dim)
    };
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(color).size(12.0))
            .fill(fill)
            .min_size(Vec2::new(0.0, 26.0)),
    )
}

/// 主按钮（实心强调色）：设计稿里「复制全部」是面板上唯一的实心按钮。
fn primary_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).size(12.0))
            .fill(theme.accent)
            .min_size(Vec2::new(0.0, 28.0)),
    )
}

/// 行号徽标。
fn badge(ui: &mut egui::Ui, theme: &Theme, number: usize) {
    let galley = ui.painter().layout_no_wrap(
        number.to_string(),
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
    let padding = Vec2::new(5.0, 2.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), theme.accent);
    painter.galley(rect.min + padding, galley, Color32::WHITE);
}

/// 底栏：左边行数与耗时，右边「重新识别 / 复制全部」。
fn footer(ui: &mut egui::Ui, theme: &Theme, lines: usize, ms: u64, action: &mut Option<OcrAction>) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(fill!(tr!("识别 {} 行，用时 {} ms"), lines, ms))
                .size(12.0)
                .color(theme.on_surface_dim),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if primary_button(ui, theme, tr!("复制全部")).clicked() {
                *action = Some(OcrAction::Copy);
            }
            if ui
                .add(egui::Button::new(tr!("重新识别")).min_size(Vec2::new(0.0, 28.0)))
                .clicked()
            {
                *action = Some(OcrAction::Retry);
            }
        });
    });
}

/// 面板窗口标题用的提示（宿主在创建视口时用）。
/// 窗口标题。跟着界面语言走，所以是个函数而不是常量。
#[must_use]
pub fn window_title() -> &'static str {
    tr!("Cuter 识别结果")
}
