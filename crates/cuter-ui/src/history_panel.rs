//! 截图历史面板。
//!
//! ## 为什么值得占一个窗口
//!
//! 截图工具最常见的动作是"刚才那张再给我一次"：刚复制完就粘错了地方、或者想换一种
//! 输出方式（复制改成保存、保存改成贴图）。没有历史，用户只能重新截一遍。
//!
//! ## 界面上的取舍
//!
//! * **一行一张**（缩略图 + 时间 / 尺寸 / 占用 + 四个动作），不做网格。
//!   网格好看，但"这张占多大、什么时候截的"这些信息在网格里没地方放，
//!   而这个面板的第二个作用恰恰是**让用户看得见磁盘占用**；
//! * **每行都有动作**，不做"先选中再操作"：少一步，也不会出现"看着选中的是 A、
//!   结果操作的是 B"；
//! * 顶部是「标题 + 张数/占用胶囊 + 打开目录 / 清空历史」：HANDOFF 第 8 节要求
//!   "缓存目录必须能一键清空"，而清空是破坏性操作，所以是"一键可达"而不是"一碰就没"
//!   （第一下变成「确定清空？」，第二下才真的清）；
//! * 版面沿用设计系统里已经定稿的那套：**卡片圆角 10、行高 72、缩略图圆角 6、
//!   按钮 28 高圆角 8**，「复制」是这一屏里唯一的实心强调按钮，「删除 / 清空」用 danger；
//! * 说明文字里写清楚**这里只有你自己的截图**，不会出现剪贴板里贴进来的图。
//!
//! 缩略图是**用到哪张读哪张**（解码后在 egui 里缓存成纹理），
//! 所以打开面板的内存开销与"当前可见的张数"有关，而不是与"历史总张数"有关。

use crate::{fill, tr};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Color32, CornerRadius, Margin, RichText};

use crate::theme::Theme;
use cuter_core::history::HistoryStore;

/// 窗口标题（宿主建视口时用它）。
/// 窗口标题。跟着界面语言走，所以是个函数而不是常量。
#[must_use]
pub fn window_title() -> &'static str {
    tr!("Cuter 截图历史")
}

/// 面板上的动作。宿主（`cuter-app`）负责真正干活——面板只发意图。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryAction {
    /// 把这一张重新复制到剪贴板。
    Copy(u64),
    /// 把这一张贴到屏幕上。
    Pin(u64),
    /// 另存为（走原生对话框）。
    SaveAs(u64),
    /// 删除这一张。
    Delete(u64),
    /// 清空全部（面板已经确认过一次）。
    Clear,
    /// 在资源管理器里打开历史目录。
    OpenDir,
    /// 关闭面板。
    Close,
}

/// 历史面板。
pub struct HistoryPanel {
    store: Arc<Mutex<HistoryStore>>,
    theme: Theme,
    /// `id → 纹理`，按需解码并缓存。
    textures: HashMap<u64, Option<egui::TextureHandle>>,
    /// 「清空」的二次确认状态。
    confirm_clear: bool,
    status: Option<String>,
}

impl HistoryPanel {
    #[must_use]
    pub fn new(store: Arc<Mutex<HistoryStore>>, theme: Theme) -> Self {
        Self {
            store,
            theme,
            textures: HashMap::new(),
            confirm_clear: false,
            status: None,
        }
    }

    /// 面板上的一行状态文字（宿主可以往里塞"已复制"之类的反馈）。
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
    }

    /// 画一帧，返回用户想做的一件事。
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<HistoryAction> {
        let mut action = None;
        let (count, total, entries) = {
            let store = lock(&self.store);
            (store.len(), store.total_bytes(), store.entries().to_vec())
        };
        let theme = self.theme;

        // ---- 顶栏：标题 + 张数/占用胶囊 + 右侧动作 ----
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("截图历史")).size(20.0).strong());
            ui.add_space(6.0);
            chip(ui, &theme, &fill!(tr!("{} 张"), count));
            chip(ui, &theme, &cuter_core::history::human_bytes(total));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(status) = &self.status {
                    ui.label(RichText::new(status).size(12.0).color(theme.accent));
                }
                if self.confirm_clear {
                    if danger_button(ui, &theme, tr!("确定清空？")) {
                        self.confirm_clear = false;
                        action = Some(HistoryAction::Clear);
                    }
                    if outline_button(ui, &theme, tr!("取消")) {
                        self.confirm_clear = false;
                    }
                } else {
                    if danger_button(ui, &theme, tr!("清空历史")) {
                        self.confirm_clear = true;
                    }
                    if outline_button(ui, &theme, tr!("打开目录")) {
                        action = Some(HistoryAction::OpenDir);
                    }
                    if outline_button(ui, &theme, tr!("关闭")) {
                        action = Some(HistoryAction::Close);
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(tr!("这里只有你自己的截图（剪贴板里贴进来的图不会进来）。上下限与缩略图大小在「历史」设置里。"))
                .size(11.0)
                .color(theme.on_surface_dim),
        );
        ui.add_space(8.0);

        if entries.is_empty() {
            egui::Frame::new()
                .fill(theme.surface_raised)
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(tr!("还没有历史。按 Alt+C 截一张就会出现在这里。"))
                            .size(13.0)
                            .color(theme.on_surface_dim),
                    );
                });
            return action;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &entries {
                let hovered = row_card(ui, &theme, |ui| {
                    ui.horizontal(|ui| {
                    // 缩略图：按需解码 + 缓存。解码失败就画个占位方块，
                    // 不让一张坏图把整个面板拖垮。
                    let key = entry.id;
                    let texture = self
                        .textures
                        .entry(key)
                        .or_insert_with(|| {
                            let store = lock(&self.store);
                            store.load_thumb_rgba(key).map(|(w, h, rgba)| {
                                let image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &rgba,
                                );
                                ui.ctx().load_texture(
                                    format!("history-thumb-{key}"),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                )
                            })
                        })
                        .clone();
                    match texture {
                        Some(texture) => {
                            let size = texture.size_vec2();
                            // 缩略图固定高度（宽度按比例），行高才不会参差不齐
                            let scale = (44.0 / size.y).min(160.0 / size.x);
                            ui.add(
                                egui::Image::new(&texture)
                                    .fit_to_exact_size(size * scale)
                                    .corner_radius(CornerRadius::same(6)),
                            );
                        }
                        None => {
                            ui.allocate_space(egui::Vec2::new(64.0, 44.0));
                            ui.label(
                                RichText::new(tr!("缩略图不可用"))
                                    .size(11.0)
                                    .color(theme.danger),
                            );
                        }
                    }

                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.created_text).size(13.0));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                entry.size_text(),
                                entry.bytes_text()
                            ))
                            .size(11.0)
                            .color(theme.on_surface_dim),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if danger_button(ui, &theme, tr!("删除")) {
                            action = Some(HistoryAction::Delete(entry.id));
                        }
                        if outline_button(ui, &theme, tr!("另存为…")) {
                            action = Some(HistoryAction::SaveAs(entry.id));
                        }
                        if outline_button(ui, &theme, tr!("贴图")) {
                            action = Some(HistoryAction::Pin(entry.id));
                        }
                        if primary_button(ui, &theme, tr!("复制")) {
                            action = Some(HistoryAction::Copy(entry.id));
                        }
                    });
                    });
                });
                let _ = hovered;
                ui.add_space(8.0);
            }
        });

        action
    }
}

/// 顶栏上的元信息胶囊（和 OCR 面板、设置页同一套观感）。
fn chip(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    egui::Frame::new()
        .fill(theme.label_bg)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(theme.on_surface_dim));
        });
}

/// 一条记录的外框（卡片）。返回这一帧是否被鼠标划过。
fn row_card<R>(ui: &mut egui::Ui, theme: &Theme, body: impl FnOnce(&mut egui::Ui) -> R) -> bool {
    let frame = egui::Frame::new()
        .fill(theme.surface_raised)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10));
    let response = frame.show(ui, body).response;
    // 卡片本身不吃点击，只借它的矩形画 hover 高亮
    let hovered = ui
        .interact(
            response.rect,
            ui.id().with(("history-row", response.rect.top() as i32)),
            egui::Sense::hover(),
        )
        .hovered();
    if hovered {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(10),
            egui::Stroke::new(1.0, theme.accent_subtle),
            egui::StrokeKind::Inside,
        );
    }
    hovered
}

/// 描边按钮（次要动作）。
fn outline_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> bool {
    let height = 28.0;
    let width = (text.chars().count() as f32 * 13.0 + 22.0).max(56.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::Vec2::new(width, height),
        egui::Sense::click(),
    );
    let fill = if response.hovered() {
        theme.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect(rect, CornerRadius::same(8), fill, egui::Stroke::new(1.0, theme.outline), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(12.0),
        theme.on_surface,
    );
    response.clicked()
}

/// 实心强调按钮（这一屏里只应该有「复制」一个）。
fn primary_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> bool {
    let height = 28.0;
    let width = (text.chars().count() as f32 * 13.0 + 22.0).max(56.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::Vec2::new(width, height),
        egui::Sense::click(),
    );
    let fill = if response.hovered() {
        theme.accent_hover
    } else {
        theme.accent
    };
    ui.painter()
        .rect(rect, CornerRadius::same(8), fill, egui::Stroke::NONE, egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(12.0),
        Color32::WHITE,
    );
    response.clicked()
}

/// 只留文字的破坏性按钮（删除 / 清空）。
fn danger_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> bool {
    let height = 28.0;
    let width = (text.chars().count() as f32 * 13.0 + 18.0).max(48.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::Vec2::new(width, height),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect(
            rect,
            CornerRadius::same(8),
            theme.danger.gamma_multiply(0.18),
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(12.0),
        theme.danger,
    );
    response.clicked()
}

/// 拿锁但不 panic：面板里的锁只保护一个本地缓存，毒化不该让界面挂掉。
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
