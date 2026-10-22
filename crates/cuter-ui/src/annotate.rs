//! 标注模型与文档。
//!
//! ## 坐标系：为什么标注挂在「冻结帧」上而不是「选区」上
//!
//! 用户框选完之后往往还要**移动/拉伸**选区。如果标注坐标相对选区，那么每次移动
//! 选区都得决定「标注跟着走还是留在原地」，并且要重新裁剪底图，语义很别扭。
//!
//! 本实现把 [`Document::base`] 定义成**整张冻结桌面帧**，标注坐标就是
//! 「桌面帧局部坐标」。于是：
//!
//! * 移动/缩放选区**不需要修改任何标注**，只是换了一个裁剪窗口；
//! * 马赛克/模糊直接作用在冻结帧上，导出时裁剪自然带上效果；
//! * 不需要在内存里再复制一份桌面图（8K 多屏下这是 60 MB 级别的差异）。
//!
//! 导出时：`base.crop(选区)` + 把标注整体平移 `-选区.origin`，越界部分由栅格化器
//! 自动裁剪（见 [`crate::flatten`]）。
//!
//! ## 为什么马赛克/模糊不进 `annotations`
//!
//! 马赛克与模糊是**像素级破坏性效果**，必须作用在底图上；而矩形、箭头等是矢量
//! 图形，绘制在上层。把两者分开带来两个好处：
//!
//! 1. 导出时只需栅格化矢量图形，像素效果已经是最终像素；
//! 2. 撤回只需保存被改动区域的那一小块像素（见 [`UndoEntry::Pixels`]），
//!    4K 截图下一次马赛克的撤销内存开销是几十 KB 而不是 33 MB。

use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect as Erect, Stroke, StrokeKind, Vec2,
};

use cuter_core::config::ToolKind;
use cuter_core::frame::Frame;
use cuter_core::geom::{Point, Rect, Size};

/// 一个矢量标注。
#[derive(Clone, Debug, PartialEq)]
pub enum Annotation {
    Rect {
        rect: Erect,
        stroke: Stroke,
        fill: Option<Color32>,
        rounding: f32,
    },
    Ellipse {
        rect: Erect,
        stroke: Stroke,
        fill: Option<Color32>,
    },
    Arrow {
        from: Pos2,
        to: Pos2,
        stroke: Stroke,
        /// 箭头头部长度（像素）。
        head_len: f32,
    },
    /// 自由画笔。
    Freehand {
        points: Vec<Pos2>,
        stroke: Stroke,
    },
    /// 荧光笔（半透明、粗、圆头）。
    Highlighter {
        points: Vec<Pos2>,
        color: Color32,
        width: f32,
    },
    Text {
        pos: Pos2,
        text: String,
        color: Color32,
        size: f32,
    },
    /// 序号标记（①②③…）。
    Counter {
        center: Pos2,
        index: u32,
        color: Color32,
        radius: f32,
    },
}

impl Annotation {
    /// 用 egui 画笔绘制到屏幕上（`offset` 为局部 → 全局的平移量）。
    pub fn paint(&self, painter: &Painter, offset: Vec2) {
        let o = |p: Pos2| p + offset;
        match self {
            Self::Rect {
                rect,
                stroke,
                fill,
                rounding,
            } => {
                let rect = rect.translate(offset);
                let corner = CornerRadius::same(rounding.round().clamp(0.0, 255.0) as u8);
                if let Some(fill) = fill {
                    painter.rect_filled(rect, corner, *fill);
                }
                painter.rect_stroke(rect, corner, *stroke, StrokeKind::Middle);
            }
            Self::Ellipse { rect, stroke, fill } => {
                let rect = rect.translate(offset);
                let center = rect.center();
                let radius = rect.size() * 0.5;
                if let Some(fill) = fill {
                    painter.add(egui::Shape::ellipse_filled(center, radius, *fill));
                }
                painter.add(egui::Shape::ellipse_stroke(center, radius, *stroke));
            }
            Self::Arrow {
                from,
                to,
                stroke,
                head_len,
            } => {
                let (from, to) = (o(*from), o(*to));
                painter.line_segment([from, to], *stroke);
                let dir = to - from;
                let len = dir.length();
                if len > 1.0 {
                    let unit = dir / len;
                    let head_len = head_len.min(len * 0.6);
                    let normal = Vec2::new(-unit.y, unit.x);
                    // 两条后掠的箭头边
                    for side in [1.0_f32, -1.0] {
                        let tip_side = to - unit * head_len + normal * (head_len * 0.45 * side);
                        painter.line_segment([to, tip_side], *stroke);
                    }
                }
            }
            Self::Freehand { points, stroke } => {
                if points.len() == 1 {
                    painter.circle_filled(o(points[0]), stroke.width * 0.5, stroke.color);
                } else if points.len() > 1 {
                    let pts: Vec<Pos2> = points.iter().map(|p| o(*p)).collect();
                    painter.add(egui::Shape::line(pts, *stroke));
                }
            }
            Self::Highlighter {
                points,
                color,
                width,
            } => {
                if points.len() > 1 {
                    let pts: Vec<Pos2> = points.iter().map(|p| o(*p)).collect();
                    painter.add(egui::Shape::line(
                        pts,
                        Stroke::new(*width, *color),
                    ));
                }
            }
            Self::Text {
                pos,
                text,
                color,
                size,
            } => {
                painter.text(
                    o(*pos),
                    Align2::LEFT_TOP,
                    text,
                    FontId::proportional(*size),
                    *color,
                );
            }
            Self::Counter {
                center,
                index,
                color,
                radius,
            } => {
                let c = o(*center);
                painter.circle_filled(c, *radius, *color);
                painter.text(
                    c,
                    Align2::CENTER_CENTER,
                    index.to_string(),
                    FontId::proportional(*radius * 1.25),
                    Color32::WHITE,
                );
            }
        }
    }

    /// 该标注影响的区域（局部坐标），用于局部重绘优化。
    #[must_use]
    pub fn bounds(&self) -> Erect {
        match self {
            Self::Rect { rect, stroke, .. } | Self::Ellipse { rect, stroke, .. } => {
                rect.expand(stroke.width * 0.5 + 1.0)
            }
            Self::Arrow {
                from,
                to,
                stroke,
                head_len,
            } => Erect::from_two_pos(*from, *to).expand(stroke.width * 0.5 + head_len * 0.5),
            Self::Freehand { points, stroke } => {
                bounding_points(points).expand(stroke.width * 0.5 + 1.0)
            }
            Self::Highlighter { points, width, .. } => {
                bounding_points(points).expand(*width * 0.5 + 1.0)
            }
            Self::Text {
                pos, size, text, ..
            } => Erect::from_min_size(
                *pos,
                Vec2::new(text.chars().count() as f32 * size * 0.62, *size * 1.35),
            ),
            Self::Counter {
                center, radius, ..
            } => Erect::from_center_size(*center, Vec2::splat(radius * 2.0 + 2.0)),
        }
    }

    /// 整体平移（用于把画布内容随选区一起移动）。
    pub fn translate(&mut self, delta: Vec2) {
        match self {
            Self::Rect { rect, .. } | Self::Ellipse { rect, .. } => *rect = rect.translate(delta),
            Self::Arrow { from, to, .. } => {
                *from += delta;
                *to += delta;
            }
            Self::Freehand { points, .. } | Self::Highlighter { points, .. } => {
                for p in points.iter_mut() {
                    *p += delta;
                }
            }
            Self::Text { pos, .. } => *pos += delta,
            Self::Counter { center, .. } => *center += delta,
        }
    }
}

fn bounding_points(points: &[Pos2]) -> Erect {
    match points.first() {
        None => Erect::NOTHING,
        Some(first) => points
            .iter()
            .skip(1)
            .fold(Erect::from_min_max(*first, *first), |acc, p| {
                acc.union(Erect::from_min_max(*p, *p))
            }),
    }
}

/// 撤回栈条目。
///
/// 矢量标注条目直接**持有**被加入的标注本身，这样重做时无需猜测内容，
/// 也让「撤回 → 重做」在两种效果上都成立。
#[derive(Clone, Debug)]
enum UndoEntry {
    /// 新增了一个矢量标注（持有副本以便重做）。
    Annotation(Annotation),
    /// 还原一块像素（马赛克 / 模糊）。
    Pixels {
        rect: Rect,
        before: Vec<u8>,
        after: Vec<u8>,
    },
}

/// 一次编辑会话的完整状态：底图 + 标注 + 撤回栈。
/// 毛玻璃底图的压暗比例：和遮罩盖在桌面上的那层（`Color32::from_gray(115)`）保持一致。
pub const FROST_DIM: f32 = 115.0 / 255.0;

pub struct Document {
    /// 文档左上角在虚拟桌面中的全局物理坐标。
    origin: Point,
    /// 底图（已被马赛克/模糊修改）。
    base: Frame,
    annotations: Vec<Annotation>,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    texture: Option<egui::TextureHandle>,
    /// 面板底下的毛玻璃底图（冻结帧不变，所以只算一次）。
    frost: Option<egui::TextureHandle>,
    texture_dirty: bool,
    /// 序号标记的下一个编号。
    next_counter: u32,
    /// 撤回栈上限（来自配置，避免长会话把内存吃光）。
    undo_limit: usize,
}

impl Document {
    /// 以一块已裁剪的图创建文档。
    #[must_use]
    pub fn new(origin: Point, base: Frame) -> Self {
        Self {
            origin,
            base,
            annotations: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            texture: None,
            frost: None,
            texture_dirty: true,
            next_counter: 1,
            undo_limit: 100,
        }
    }

    /// 设置撤回栈上限；超出部分按最旧的丢弃。
    ///
    /// 之所以需要上限：几十次「模糊整屏」这类像素效果的撤销条目各自持有一块像素，
    /// 无上限时长时间编辑会把内存撑到几百 MB。
    pub fn set_undo_limit(&mut self, limit: usize) {
        self.undo_limit = limit.clamp(1, 1000);
        self.trim_undo();
    }

    fn trim_undo(&mut self) {
        while self.undo_stack.len() > self.undo_limit {
            self.undo_stack.remove(0);
        }
        while self.redo_stack.len() > self.undo_limit {
            self.redo_stack.remove(0);
        }
    }

    #[must_use]
    pub fn origin(&self) -> Point {
        self.origin
    }

    /// 局部坐标 → 全局坐标的平移量。
    #[must_use]
    pub fn offset(&self) -> Vec2 {
        Vec2::new(self.origin.x as f32, self.origin.y as f32)
    }

    #[must_use]
    pub fn size(&self) -> Size {
        self.base.size()
    }

    #[must_use]
    pub fn base(&self) -> &Frame {
        &self.base
    }

    #[must_use]
    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    #[must_use]
    pub fn local_bounds(&self) -> Erect {
        Erect::from_min_size(Pos2::ZERO, Vec2::new(self.size().w as f32, self.size().h as f32))
    }

    /// 文档在全局坐标下的矩形。
    #[must_use]
    pub fn global_bounds(&self) -> Rect {
        Rect::new(
            self.origin.x,
            self.origin.y,
            self.size().w,
            self.size().h,
        )
    }

    pub fn to_global(&self, local: Pos2) -> Pos2 {
        local + self.offset()
    }

    pub fn to_local(&self, global: Pos2) -> Pos2 {
        global - self.offset()
    }

    // ------------------------------------------------------------------
    // 纹理
    // ------------------------------------------------------------------

    /// 取（必要时重建）底图纹理。
    /// 面板底下那层毛玻璃（懒加载：整帧缩到 1/4、两遍盒式模糊、叠白 10%、压暗到和遮罩一致）。
    ///
    /// 冻结帧在整场遮罩会话里不变，所以这里只算一次；`base` 换新时请重建 `Document`。
    pub fn frost_texture(&mut self, ctx: &egui::Context) -> &egui::TextureHandle {
        if self.frost.is_none() {
            let small = crate::frost::frost(&self.base, 0.25, 6, 2, 0.10, FROST_DIM);
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [small.width() as usize, small.height() as usize],
                small.data(),
            );
            self.frost = Some(ctx.load_texture(
                "cuter-frost",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        self.frost.as_ref().expect("frost 刚在上面建过")
    }

    pub fn texture(&mut self, ctx: &egui::Context) -> &egui::TextureHandle {
        if self.texture_dirty || self.texture.is_none() {
            let image = frame_to_color_image(&self.base);
            match &mut self.texture {
                Some(handle) => handle.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.texture = Some(ctx.load_texture(
                        "cuter-document",
                        image,
                        egui::TextureOptions::NEAREST,
                    ));
                }
            }
            self.texture_dirty = false;
        }
        self.texture.as_ref().expect("刚刚写入过")
    }

    // ------------------------------------------------------------------
    // 编辑
    // ------------------------------------------------------------------

    /// 追加矢量标注。
    pub fn push_annotation(&mut self, annotation: Annotation) {
        self.undo_stack
            .push(UndoEntry::Annotation(annotation.clone()));
        self.annotations.push(annotation);
        self.redo_stack.clear();
        self.trim_undo();
    }

    /// 申请一个序号标记编号。
    pub fn take_counter_index(&mut self) -> u32 {
        let i = self.next_counter;
        self.next_counter += 1;
        i
    }

    /// 追加文字标注。
    pub fn push_text(&mut self, pos: Pos2, text: String, color: Color32, size: f32) {
        if text.is_empty() {
            return;
        }
        self.push_annotation(Annotation::Text {
            pos,
            text,
            color,
            size,
        });
    }

    /// 对局部矩形施加马赛克（可撤回）。
    pub fn apply_mosaic(&mut self, rect: Rect, block: u32) {
        self.apply_pixel_effect(rect, |frame, rect| frame.mosaic(rect, block));
    }

    /// 对局部矩形施加模糊（可撤回）。
    pub fn apply_blur(&mut self, rect: Rect, radius: u32, passes: u32) {
        self.apply_pixel_effect(rect, |frame, rect| frame.blur(rect, radius, passes));
    }

    fn apply_pixel_effect(&mut self, rect: Rect, effect: impl FnOnce(&mut Frame, Rect)) {
        let rect = rect.intersect(self.base.bounds());
        if rect.is_empty() {
            return;
        }
        let before = self.base.crop(rect).into_data();
        effect(&mut self.base, rect);
        let after = self.base.crop(rect).into_data();
        if before == after {
            return;
        }
        self.undo_stack.push(UndoEntry::Pixels { rect, before, after });
        self.redo_stack.clear();
        self.texture_dirty = true;
        self.trim_undo();
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo_stack.pop() else {
            return false;
        };
        match &entry {
            UndoEntry::Annotation(_) => {
                self.annotations.pop();
            }
            UndoEntry::Pixels { rect, before, .. } => {
                self.restore_pixels(*rect, before);
            }
        }
        self.redo_stack.push(entry);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo_stack.pop() else {
            return false;
        };
        match &entry {
            UndoEntry::Annotation(annotation) => {
                self.annotations.push(annotation.clone());
            }
            UndoEntry::Pixels { rect, after, .. } => {
                self.restore_pixels(*rect, after);
            }
        }
        self.undo_stack.push(entry);
        true
    }

    fn restore_pixels(&mut self, rect: Rect, bytes: &[u8]) {
        if let Ok(patch) = Frame::from_rgba(rect.size(), bytes.to_vec()) {
            self.base.blit(&patch, rect.origin());
            self.texture_dirty = true;
        }
    }

    /// 清空所有标注（保留底图）。
    pub fn clear_annotations(&mut self) {
        if self.annotations.is_empty() {
            return;
        }
        self.annotations.clear();
        self.undo_stack
            .retain(|e| !matches!(e, UndoEntry::Annotation(_)));
        self.redo_stack.clear();
    }

    /// 放弃整块文档（用于切换选区）。
    pub fn reset(&mut self, origin: Point, base: Frame) {
        *self = Self::new(origin, base);
    }

    // ------------------------------------------------------------------
    // 绘制
    // ------------------------------------------------------------------

    /// 只绘制标注（不绘制底图——底图由调用方按需带明暗地绘制，见遮罩实现）。
    ///
    /// `offset` 是「文档坐标 → 目标绘制坐标系」的平移量。遮罩里两者同系，
    /// 传 [`Vec2::ZERO`]；独立预览窗口可以传别的位置。
    pub fn paint_annotations(&self, painter: &Painter, offset: Vec2) {
        // 标注可能被拖到选区之外，交给 egui 的裁剪矩形处理
        let clipped = painter.with_clip_rect(painter.clip_rect());
        for a in &self.annotations {
            a.paint(&clipped, offset);
        }
    }
}

/// `Frame` → egui 纹理数据。
///
/// 屏幕截图恒为不透明，因此可以直接按「预乘 = 直通」构造 `Color32`，
/// 省掉 `from_rgba_unmultiplied` 的逐像素乘法；8M 像素下这能省下约 30 ms。
#[must_use]
pub fn frame_to_color_image(frame: &Frame) -> egui::ColorImage {
    let size = frame.size();
    let pixels: Vec<Color32> = frame
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    egui::ColorImage {
        size: [size.w as usize, size.h as usize],
        pixels,
        source_size: Vec2::new(size.w as f32, size.h as f32),
    }
}

/// 工具参数集合：当前工具、颜色、线宽等，工具条直接编辑它。
///
/// `PartialEq` 是给"弹窗里改过样式吗"用的：改过才写配置，避免每帧都写盘。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    pub tool: ToolKind,
    pub color: Color32,
    pub stroke_width: f32,
    pub fill_shapes: bool,
    pub font_size: f32,
    pub mosaic_block: u32,
    pub blur_radius: u32,
    pub counter_radius: f32,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            tool: ToolKind::Rectangle,
            color: Color32::from_rgb(255, 59, 48),
            stroke_width: 3.0,
            fill_shapes: false,
            font_size: 20.0,
            mosaic_block: 12,
            blur_radius: 10,
            counter_radius: 14.0,
        }
    }
}

impl ToolSettings {
    /// 从配置初始化。
    #[must_use]
    pub fn from_config(cfg: &cuter_core::config::AnnotateConfig) -> Self {
        Self {
            tool: cfg.default_tool,
            color: Color32::from_rgb(
                cfg.stroke_color[0],
                cfg.stroke_color[1],
                cfg.stroke_color[2],
            ),
            stroke_width: cfg.stroke_width,
            fill_shapes: cfg.fill_shapes,
            font_size: cfg.font_size,
            mosaic_block: cfg.mosaic_block,
            blur_radius: cfg.blur_radius,
            counter_radius: 14.0,
        }
    }

    #[must_use]
    pub fn stroke(&self) -> Stroke {
        Stroke::new(self.stroke_width, self.color)
    }

    /// 荧光笔颜色：原色降低不透明度并提高亮度。
    #[must_use]
    pub fn highlighter_color(&self) -> Color32 {
        let [r, g, b, _] = self.color.to_array();
        Color32::from_rgba_unmultiplied(r, g, b, 110)
    }

    #[must_use]
    pub fn fill_color(&self) -> Option<Color32> {
        if self.fill_shapes {
            let [r, g, b, _] = self.color.to_array();
            Some(Color32::from_rgba_unmultiplied(r, g, b, 60))
        } else {
            None
        }
    }
}

/// 常用颜色板（工具条取色弹窗）。
pub const PALETTE: [Color32; 16] = [
    Color32::from_rgb(255, 59, 48),
    Color32::from_rgb(255, 149, 0),
    Color32::from_rgb(255, 204, 0),
    Color32::from_rgb(52, 199, 89),
    Color32::from_rgb(0, 199, 190),
    Color32::from_rgb(0, 122, 255),
    Color32::from_rgb(88, 86, 214),
    Color32::from_rgb(175, 82, 222),
    Color32::WHITE,
    Color32::from_rgb(230, 230, 230),
    Color32::from_rgb(150, 150, 150),
    Color32::from_rgb(80, 80, 80),
    Color32::from_rgb(30, 30, 30),
    Color32::BLACK,
    Color32::from_rgb(255, 45, 85),
    Color32::from_rgb(162, 132, 94),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 有纹理的底图：马赛克/模糊这类像素效果**必须**在有变化可观测的图上测，
    /// 纯色底图上做马赛克本来就不产生任何差异。
    fn doc() -> Document {
        let size = Size::new(40, 30);
        let mut base = Frame::new(size);
        for y in 0..size.h as i32 {
            for x in 0..size.w as i32 {
                let v = ((x * 7 + y * 13) % 251) as u8;
                base.set_pixel(x, y, [v, v.wrapping_add(97), v.wrapping_add(53), 255]);
            }
        }
        Document::new(Point::new(-100, 50), base)
    }

    /// 纯色底图：用来验证「无变化就不该产生撤回条目」。
    fn uniform_doc() -> Document {
        Document::new(
            Point::new(-100, 50),
            Frame::filled(Size::new(40, 30), [10, 20, 30, 255]),
        )
    }

    #[test]
    fn global_local_roundtrip_uses_origin() {
        let d = doc();
        assert_eq!(d.to_global(Pos2::new(1.0, 2.0)), Pos2::new(-99.0, 52.0));
        assert_eq!(d.to_local(Pos2::new(-99.0, 52.0)), Pos2::new(1.0, 2.0));
        assert_eq!(d.global_bounds(), Rect::new(-100, 50, 40, 30));
    }

    #[test]
    fn mosaic_is_undoable_and_redoable() {
        let mut d = doc();
        let original = d.base().clone();
        d.apply_mosaic(Rect::new(0, 0, 40, 30), 8);
        assert!(d.can_undo());
        assert_ne!(d.base().data(), original.data());

        assert!(d.undo());
        assert_eq!(d.base().data(), original.data());
        assert!(!d.can_undo());
        assert!(d.can_redo());

        assert!(d.redo());
        assert_ne!(d.base().data(), original.data());
    }

    #[test]
    fn mosaic_on_uniform_area_is_a_noop() {
        let mut d = uniform_doc();
        // 纯色区域做马赛克不会改变像素，不应产生撤回条目
        d.apply_mosaic(Rect::new(5, 5, 10, 10), 4);
        assert!(!d.can_undo(), "没有实际变化时不应压入撤回栈");
    }

    #[test]
    fn out_of_bounds_effect_is_ignored() {
        let mut d = doc();
        d.apply_mosaic(Rect::new(500, 500, 10, 10), 4);
        assert!(!d.can_undo());
    }

    #[test]
    fn annotations_are_undone_in_lifo_order() {
        let mut d = doc();
        d.push_annotation(Annotation::Rect {
            rect: Erect::from_min_size(Pos2::ZERO, Vec2::splat(5.0)),
            stroke: Stroke::new(1.0, Color32::RED),
            fill: None,
            rounding: 0.0,
        });
        d.push_annotation(Annotation::Arrow {
            from: Pos2::ZERO,
            to: Pos2::new(10.0, 10.0),
            stroke: Stroke::new(1.0, Color32::RED),
            head_len: 8.0,
        });
        assert_eq!(d.annotations().len(), 2);
        d.undo();
        assert_eq!(d.annotations().len(), 1);
        d.undo();
        assert!(d.annotations().is_empty());
        assert!(!d.undo());
    }

    #[test]
    fn counter_indices_are_monotonic() {
        let mut d = doc();
        assert_eq!(d.take_counter_index(), 1);
        assert_eq!(d.take_counter_index(), 2);
    }

    #[test]
    fn undo_limit_drops_the_oldest_entries() {
        let mut d = doc();
        d.set_undo_limit(3);
        for i in 0..6 {
            d.push_annotation(Annotation::Counter {
                center: Pos2::new(i as f32, 0.0),
                index: i + 1,
                color: Color32::RED,
                radius: 5.0,
            });
        }
        assert_eq!(d.annotations().len(), 6, "上限只影响撤回能力，不影响已有标注");

        let mut undone = 0;
        while d.undo() {
            undone += 1;
        }
        assert_eq!(undone, 3, "撤回次数应当被上限限制");
        assert_eq!(d.annotations().len(), 3, "最旧的 3 条应当已经无法撤回");
        assert!(!d.can_undo());
        assert!(d.can_redo());
    }

    #[test]
    fn undo_limit_is_clamped_and_never_zero() {
        let mut d = doc();
        d.set_undo_limit(0);
        // 0 会被夹到 1，否则撤销功能会彻底失效
        d.push_annotation(Annotation::Counter {
            center: Pos2::ZERO,
            index: 1,
            color: Color32::RED,
            radius: 5.0,
        });
        assert!(d.undo());
    }

    #[test]
    fn set_undo_limit_trims_existing_history() {
        let mut d = doc();
        for i in 0..5 {
            d.push_annotation(Annotation::Counter {
                center: Pos2::new(i as f32, 0.0),
                index: i + 1,
                color: Color32::RED,
                radius: 5.0,
            });
        }
        d.set_undo_limit(2);
        let mut undone = 0;
        while d.undo() {
            undone += 1;
        }
        assert_eq!(undone, 2, "调小上限应当立刻裁掉旧历史");
    }

    #[test]
    fn translate_moves_every_variant() {
        let delta = Vec2::new(3.0, -4.0);
        let mut variants = vec![
            Annotation::Rect {
                rect: Erect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
                stroke: Stroke::new(1.0, Color32::RED),
                fill: None,
                rounding: 0.0,
            },
            Annotation::Ellipse {
                rect: Erect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
                stroke: Stroke::new(1.0, Color32::RED),
                fill: None,
            },
            Annotation::Arrow {
                from: Pos2::ZERO,
                to: Pos2::new(1.0, 1.0),
                stroke: Stroke::new(1.0, Color32::RED),
                head_len: 2.0,
            },
            Annotation::Freehand {
                points: vec![Pos2::ZERO, Pos2::new(1.0, 1.0)],
                stroke: Stroke::new(1.0, Color32::RED),
            },
            Annotation::Highlighter {
                points: vec![Pos2::ZERO],
                color: Color32::YELLOW,
                width: 5.0,
            },
            Annotation::Text {
                pos: Pos2::ZERO,
                text: "x".into(),
                color: Color32::RED,
                size: 12.0,
            },
            Annotation::Counter {
                center: Pos2::ZERO,
                index: 1,
                color: Color32::RED,
                radius: 8.0,
            },
        ];
        for v in &mut variants {
            v.translate(delta);
        }
        // `bounds()` 会在几何之外再外扩 `stroke.width/2 + 1.0`（给抗锯齿留边），
        // 所以期望值必须把这个外扩一起算上。
        let expected = Erect::from_min_size(Pos2::ZERO, Vec2::splat(4.0))
            .translate(delta)
            .expand(1.5);
        assert_eq!(variants[0].bounds().min, expected.min);
        assert_eq!(variants[0].bounds().max, expected.max);
        assert_eq!(
            match &variants[6] {
                Annotation::Counter { center, .. } => *center,
                _ => unreachable!(),
            },
            Pos2::new(3.0, -4.0)
        );
    }

    #[test]
    fn bounds_of_empty_freehand_is_nothing() {
        let a = Annotation::Freehand {
            points: vec![],
            stroke: Stroke::new(1.0, Color32::RED),
        };
        assert!(a.bounds().is_negative());
    }

    #[test]
    fn color_image_matches_frame_size() {
        let f = Frame::filled(Size::new(7, 5), [1, 2, 3, 255]);
        let img = frame_to_color_image(&f);
        assert_eq!(img.size, [7, 5]);
        assert_eq!(img.pixels.len(), 35);
        assert_eq!(img.pixels[0], Color32::from_rgba_premultiplied(1, 2, 3, 255));
    }
}
