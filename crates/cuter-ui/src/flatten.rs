//! 把矢量标注真正画进像素（导出 PNG / 复制到剪贴板的必要一步）。
//!
//! ## 为什么需要这一层
//!
//! 屏幕上 [`Annotation`] 是由 egui 直接画到 GPU 的，导出时拿不到那些像素。
//! 所以这里实现一个**只依赖 `ab_glyph` 的小型软件光栅器**，把同一份标注数据
//! 以与 `annotate.rs` 预览**相同的几何**烘焙进 [`Frame`] 的副本里。
//!
//! ## 坐标与颜色约定
//!
//! * 所有坐标都是**物理像素**：`Annotation` 里的 `Pos2` / `Rect` 已经是像素坐标，
//!   因此 `1.0 == 1 像素`，栅格化时不存在任何缩放因子。
//! * 像素 `(x, y)` 覆盖区域 `[x, x + 1] × [y, y + 1]`，采样点取**像素中心**
//!   `(x + 0.5, y + 0.5)`。
//! * [`Frame`] 的 RGBA8 与 `egui::Color32` 一样是**预乘**的，因此 source-over 为
//!   `out_rgb = src_rgb·c + dst_rgb·(1 - src_a·c)`、
//!   `out_a = src_a·c + dst_a·(1 - src_a·c)`（`c` 为覆盖率）。
//!   注意 `Color32` 的分量**已经是预乘值**，所以只需再乘一次覆盖率，
//!   **不要**再乘 alpha——那会把 alpha 算两遍，半透明荧光笔会变得过暗。
//!
//! ## 抗锯齿
//!
//! 全部图元都用**距离场 + 1 像素线性斜坡**：
//!
//! ```text
//! 填充：coverage = clamp(0.5 - d,        0, 1)   // d 为有符号距离，内部为负
//! 描边：coverage = clamp(0.5 + w/2 - |d|, 0, 1)   // |d| 为到轮廓线的距离
//! ```
//!
//! `+0.5` 让过渡带正好是一像素宽并**以几何边界为中心**（像素中心采样时，
//! 覆盖率 0.5 恰好落在边界上），这与 egui 的 feathered tessellation 观感一致。
//! 文字则由 `ab_glyph` 自己给出覆盖率，直接把覆盖率交给同一个混合函数。

use ab_glyph::{Font, OutlinedGlyph, PxScale, ScaleFont};
use cuter_core::frame::Frame;
use egui::{Color32, Pos2, Rect as ERect, Vec2};

use crate::annotate::Annotation;

/// 用于栅格化文字的字体（由 `cuter_ui::fonts::LoadedFont` 提供）。
#[derive(Clone, Copy, Debug)]
pub struct FontRef<'a> {
    pub bytes: &'a [u8],
    /// TrueType Collection 内的字体索引；普通 ttf 为 0。
    pub index: u32,
}

/// 把 `annotations` 烘焙到 `base` 的**副本**上并返回。
///
/// `font` 为 `None` 时文字标注退化为占位框（并 `log::warn!` 一次）；
/// 字体数据无法解析时同样降级为占位框，**不会 panic**。
///
/// 传入的 `base` 不会被修改；除 `base.clone()` 外不再分配与图幅同量级的缓冲区。
#[must_use]
pub fn flatten(base: &Frame, annotations: &[Annotation], font: Option<FontRef<'_>>) -> Frame {
    let mut frame = base.clone();

    // 每次调用最多产出一条降级日志：字体缺失在真正需要时才提醒，
    // 字体损坏则立刻提醒（并把标志置上，避免重复刷屏）。
    let mut font_warned = false;
    let parsed = font.and_then(|font| {
        match ab_glyph::FontRef::try_from_slice_and_index(font.bytes, font.index) {
            Ok(parsed) => Some(parsed),
            Err(err) => {
                log::warn!(
                    "字体数据解析失败（index {}）：{err}，文字标注将退化为占位框",
                    font.index
                );
                font_warned = true;
                None
            }
        }
    });

    let mut baker = Baker {
        frame: &mut frame,
        font: parsed,
        font_warned,
    };
    for annotation in annotations {
        baker.draw(annotation);
    }
    frame
}

// ============================================================================
// 栅格化上下文
// ============================================================================

/// 一次 `flatten` 调用的绘制上下文：目标帧 + 已解析的字体。
struct Baker<'f, 'a> {
    frame: &'f mut Frame,
    font: Option<ab_glyph::FontRef<'a>>,
    font_warned: bool,
}

impl Baker<'_, '_> {
    /// 字体不可用时提醒一次（每次 `flatten` 至多一条日志，避免刷屏）。
    fn warn_missing_font(&mut self) {
        if !self.font_warned {
            self.font_warned = true;
            log::warn!("未提供可用字体：文字标注退化为占位框，序号标注只画圆");
        }
    }

    /// 按 [`Annotation`] 变体分派，几何逐条对齐 `annotate.rs` 的 egui 预览。
    fn draw(&mut self, annotation: &Annotation) {
        match annotation {
            Annotation::Rect {
                rect,
                stroke,
                fill,
                rounding,
            } => {
                // egui 预览用 `CornerRadius::same(rounding.round() as u8)` 吸收圆角，
                // 这里取整到同样的粒度，保证预览与导出不会差出半个像素。
                let rounding = rounding.round().clamp(0.0, 255.0);
                if let Some(fill) = fill {
                    if rounding > 0.0 {
                        fill_rounded_rect(self.frame, *rect, rounding, *fill);
                    } else {
                        fill_rect(self.frame, *rect, *fill);
                    }
                }
                stroke_rect(self.frame, *rect, stroke.width, stroke.color, rounding);
            }
            Annotation::Ellipse { rect, stroke, fill } => {
                if let Some(fill) = fill {
                    fill_ellipse(self.frame, *rect, *fill);
                }
                stroke_ellipse(self.frame, *rect, stroke.width, stroke.color);
            }
            Annotation::Arrow {
                from,
                to,
                stroke,
                head_len,
            } => {
                stroke_line(self.frame, *from, *to, stroke.width, stroke.color, true);
                let dir = *to - *from;
                let len = dir.length();
                if len > 1.0 {
                    // 与 egui 预览完全相同的头部公式：
                    // unit = dir / len，normal = (-unit.y, unit.x)，
                    // 两条后掠边连到 to - unit·head + normal·(head·0.45·side)。
                    let unit = dir / len;
                    let head_len = head_len.min(len * 0.6);
                    let normal = Vec2::new(-unit.y, unit.x);
                    for side in [1.0_f32, -1.0] {
                        let tip = *to - unit * head_len + normal * (head_len * 0.45 * side);
                        stroke_line(self.frame, *to, tip, stroke.width, stroke.color, true);
                    }
                }
            }
            Annotation::Freehand { points, stroke } => {
                if points.len() == 1 {
                    fill_circle(self.frame, points[0], stroke.width * 0.5, stroke.color);
                } else if points.len() > 1 {
                    stroke_polyline(self.frame, points, stroke.width, stroke.color);
                }
            }
            Annotation::Highlighter {
                points,
                color,
                width,
            } => {
                // 与预览一致：单点不成笔（颜色自带 alpha，交给混合函数处理）。
                if points.len() > 1 {
                    stroke_polyline(self.frame, points, *width, *color);
                }
            }
            Annotation::Text {
                pos,
                text,
                color,
                size,
            } => self.text(*pos, text, *color, *size),
            Annotation::Counter {
                center,
                index,
                color,
                radius,
            } => self.counter(*center, *index, *color, *radius),
        }
    }

    /// 文字标注：`pos` 为文字**左上角**（对应 `Align2::LEFT_TOP`）。
    fn text(&mut self, pos: Pos2, text: &str, color: Color32, size: f32) {
        if text.is_empty() || !is_positive(size) {
            return;
        }
        let has_font = self.font.is_some();
        let mut drawn = false;
        if let Some(font) = self.font.as_ref() {
            let glyphs = layout_glyphs(font, text, size, pos);
            if !glyphs.is_empty() {
                draw_glyphs(self.frame, &glyphs, color);
                drawn = true;
            }
        }
        if !drawn && !has_font {
            // 降级：画一个与 `Annotation::bounds()` 同尺寸的空心占位框。
            self.warn_missing_font();
            let placeholder = ERect::from_min_size(
                pos,
                Vec2::new(text.chars().count() as f32 * size * 0.62, size * 1.35),
            );
            stroke_rect(self.frame, placeholder, 1.0, color, 0.0);
        }
    }

    /// 序号标记：实心圆 + 水平垂直都居中的白色数字。
    fn counter(&mut self, center: Pos2, index: u32, color: Color32, radius: f32) {
        if !is_positive(radius) {
            return;
        }
        fill_circle(self.frame, center, radius, color);
        if self.font.is_none() {
            // 降级：只画圆（圆形标注本身已经表达了序号位置）。
            self.warn_missing_font();
            return;
        }
        let Some(font) = self.font.as_ref() else {
            return;
        };
        let text = index.to_string();
        let size = radius * 1.25;
        // 先按原点排版量出**字形的实际像素包围盒**，再把它的中心挪到圆心：
        // 不假设数字等宽，所以 "1" 和 "8" 都能真正居中。
        let glyphs = layout_glyphs(font, &text, size, Pos2::ZERO);
        let Some(bounds) = glyph_union(&glyphs) else {
            return;
        };
        let shift = center - bounds.center();
        // 用带小数的原点重新排版，等价于带子像素精度的平移。
        let glyphs = layout_glyphs(font, &text, size, Pos2::new(shift.x, shift.y));
        draw_glyphs(self.frame, &glyphs, Color32::WHITE);
    }
}

// ============================================================================
// 像素访问与混合
// ============================================================================

/// 像素遍历范围（左闭右开，且已与帧范围求交）。
#[derive(Clone, Copy, Debug)]
struct Clip {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// 由浮点包围盒（物理像素）算出需要遍历的整数像素范围。
///
/// 非有限坐标（`NaN` / `∞`）直接返回 `None`：宁可不画，也不要把 `NaN`
/// 混进混合公式。所有图元都先过这一步，因此循环永远只跑在真正相关的像素上。
fn clip_bounds(frame: &Frame, min: Pos2, max: Pos2) -> Option<Clip> {
    if !min.x.is_finite() || !min.y.is_finite() || !max.x.is_finite() || !max.y.is_finite() {
        return None;
    }
    let x0 = min.x.floor().max(0.0);
    let y0 = min.y.floor().max(0.0);
    let x1 = max.x.ceil().min(frame.width() as f32);
    let y1 = max.y.ceil().min(frame.height() as f32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Clip {
        x0: x0 as i32,
        y0: y0 as i32,
        x1: x1 as i32,
        y1: y1 as i32,
    })
}

/// 矩形版本（会先规范化倒置矩形）。
fn clip_rect(frame: &Frame, rect: ERect) -> Option<Clip> {
    let rect = normalized(rect);
    clip_bounds(frame, rect.min, rect.max)
}

/// 把可能倒置的矩形规范化成 `min <= max`。
fn normalized(rect: ERect) -> ERect {
    ERect::from_min_max(rect.min.min(rect.max), rect.max.max(rect.min))
}

/// 严格正数判定，`NaN` 与 0、负数同等对待。
///
/// 特意**不**写成 `v <= 0.0` 的取反：`NaN <= 0.0` 是 `false`，那样 `NaN`
/// 会一路流进距离场与字形位图，所以统一用它做「提前返回」的守卫。
fn is_positive(v: f32) -> bool {
    v > 0.0
}

/// `[0, 1]` 的浮点分量 → `u8`（四舍五入 + 夹取）。
fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// 以 `coverage` 的覆盖率把 `color` 混合到像素 `(x, y)` 上（source-over）。
///
/// * `color` 是 egui 的**预乘** `Color32`，其分量只需再乘一次覆盖率；
/// * 目标像素自身的 alpha 参与运算，因此半透明底图（贴图、透明 PNG）也正确：
///   `out_a = src_a·c + dst_a·(1 - src_a·c)`。
///
/// 越界坐标、零覆盖率、`NaN` 一律静默忽略——越界写入必须绝对避免。
fn blend_pixel(frame: &mut Frame, x: i32, y: i32, color: Color32, coverage: f32) {
    if !is_positive(coverage) {
        return;
    }
    let (w, h) = (frame.width() as i32, frame.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let coverage = coverage.min(1.0);
    let [sr, sg, sb, sa] = color.to_array();
    let src_a = sa as f32 / 255.0 * coverage;
    if src_a <= 0.0 {
        return;
    }
    let src = [
        sr as f32 / 255.0 * coverage,
        sg as f32 / 255.0 * coverage,
        sb as f32 / 255.0 * coverage,
    ];
    let inv = 1.0 - src_a;
    let off = (y as usize * frame.width() as usize + x as usize) * 4;
    let data = frame.data_mut();
    for channel in 0..3 {
        let dst = data[off + channel] as f32 / 255.0;
        data[off + channel] = to_u8(src[channel] + dst * inv);
    }
    let dst_a = data[off + 3] as f32 / 255.0;
    data[off + 3] = to_u8(src_a + dst_a * inv);
}

// ============================================================================
// 距离场基础
// ============================================================================

/// 点到线段的最短距离（含两端半圆帽）。
fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= 1e-12 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// 「平头」线段的最短距离：投影落在两端之外时返回 `None`（该点完全不被覆盖）。
fn distance_to_flat_segment(p: Pos2, a: Pos2, b: Pos2) -> Option<f32> {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= 1e-12 {
        return None;
    }
    let t = (p - a).dot(ab) / len2;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some((p - (a + ab * t)).length())
}

/// 单个像素与轴对齐矩形的**精确**重叠面积（`0..=1`）。
fn box_coverage(rect: ERect, px: f32, py: f32) -> f32 {
    let cx = ((px + 0.5).min(rect.max.x) - (px - 0.5).max(rect.min.x)).clamp(0.0, 1.0);
    let cy = ((py + 0.5).min(rect.max.y) - (py - 0.5).max(rect.min.y)).clamp(0.0, 1.0);
    cx * cy
}

/// 圆角矩形的有符号距离（内部为负，`rounding` 单位为像素）。
///
/// 圆角半径按半边长夹取：`rounding` 超过半边长时几何上会退化成胶囊，
/// 夹取可以让距离场保持在有效范围内（egui 预览在这种情况下形状本来也不确定）。
fn rounded_rect_sdf(p: Pos2, rect: ERect, rounding: f32) -> f32 {
    let half = rect.size() * 0.5;
    let r = rounding.clamp(0.0, half.x.min(half.y));
    let center = rect.center();
    let qx = (p.x - center.x).abs() - (half.x - r);
    let qy = (p.y - center.y).abs() - (half.y - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// 椭圆的中心与半轴；退化（半轴 <= 0）时返回 `None`。
fn ellipse_parts(rect: ERect) -> Option<(Pos2, f32, f32)> {
    let rect = normalized(rect);
    let half = rect.size() * 0.5;
    if !is_positive(half.x) || !is_positive(half.y) {
        return None;
    }
    Some((rect.center(), half.x, half.y))
}

/// 椭圆的**近似**有符号距离：`f / |∇f|`（`f = (dx/rx)² + (dy/ry)² - 1`）。
///
/// 椭圆的真实距离没有闭式解，这个一阶泰勒展开在轮廓附近误差远小于 0.05 像素，
/// 而抗锯齿只关心轮廓附近 ±0.5 像素的范围，因此完全够用。
fn ellipse_sdf(p: Pos2, center: Pos2, rx: f32, ry: f32) -> f32 {
    let dx = p.x - center.x;
    let dy = p.y - center.y;
    let f = (dx / rx).powi(2) + (dy / ry).powi(2) - 1.0;
    let gx = 2.0 * dx / (rx * rx);
    let gy = 2.0 * dy / (ry * ry);
    let g = gx.hypot(gy);
    if g <= 1e-6 {
        // 圆心附近梯度趋零，直接沿用「到最近轮廓」的量级（符号仍为内部）。
        return -rx.min(ry);
    }
    f / g
}

/// 内缩后的矩形；缩没了返回 `None`。
fn inner_rect(rect: ERect, amount: f32) -> Option<ERect> {
    let inner = ERect::from_min_max(
        rect.min + Vec2::splat(amount),
        rect.max - Vec2::splat(amount),
    );
    (inner.max.x > inner.min.x && inner.max.y > inner.min.y).then_some(inner)
}

// ============================================================================
// 图元
// ============================================================================

/// 实心圆。
fn fill_circle(frame: &mut Frame, center: Pos2, radius: f32, color: Color32) {
    if !is_positive(radius) {
        return;
    }
    let reach = radius + 1.0;
    let Some(clip) = clip_bounds(
        frame,
        center - Vec2::splat(reach),
        center + Vec2::splat(reach),
    ) else {
        return;
    };
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let p = Pos2::new(x as f32 + 0.5, py);
            let distance = (p - center).length() - radius;
            blend_pixel(frame, x, y, color, 0.5 - distance);
        }
    }
}

/// 实心轴对齐矩形（用精确的像素覆盖面积抗锯齿，比距离场更准）。
fn fill_rect(frame: &mut Frame, rect: ERect, color: Color32) {
    let rect = normalized(rect);
    let Some(clip) = clip_rect(frame, rect) else {
        return;
    };
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let coverage = box_coverage(rect, x as f32 + 0.5, py);
            blend_pixel(frame, x, y, color, coverage);
        }
    }
}

/// 实心圆角矩形。
fn fill_rounded_rect(frame: &mut Frame, rect: ERect, rounding: f32, color: Color32) {
    let rect = normalized(rect);
    let Some(clip) = clip_rect(frame, rect.expand(1.0)) else {
        return;
    };
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let d = rounded_rect_sdf(Pos2::new(x as f32 + 0.5, py), rect, rounding);
            blend_pixel(frame, x, y, color, 0.5 - d);
        }
    }
}

/// 线段描边。`round_cap` 为真时按「胶囊」处理（两端为半圆帽）。
///
/// `coverage = clamp(w/2 + 0.5 - dist, 0, 1)`：到线段的距离不小于 `w/2`
/// 的像素被覆盖，落在边界 ±0.5 像素内的像素拿到 0..1 的中间覆盖率。
fn stroke_line(frame: &mut Frame, a: Pos2, b: Pos2, width: f32, color: Color32, round_cap: bool) {
    if !is_positive(width) {
        return;
    }
    let half = width * 0.5;
    let reach = half + 1.0;
    let Some(clip) = clip_bounds(
        frame,
        a.min(b) - Vec2::splat(reach),
        a.max(b) + Vec2::splat(reach),
    ) else {
        return;
    };
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let p = Pos2::new(x as f32 + 0.5, py);
            let distance = if round_cap {
                distance_to_segment(p, a, b)
            } else {
                match distance_to_flat_segment(p, a, b) {
                    Some(d) => d,
                    None => continue,
                }
            };
            blend_pixel(frame, x, y, color, half + 0.5 - distance);
        }
    }
}

/// 折线描边：逐段画胶囊，并在**拐点补圆**消除缺口。
///
/// 手绘曲线可能有上千个点，所以这里只做一遍线性扫描（O(n)），
/// 每个点的补圆只覆盖自己那一小块包围盒。
fn stroke_polyline(frame: &mut Frame, points: &[Pos2], width: f32, color: Color32) {
    if !is_positive(width) || points.is_empty() {
        return;
    }
    if points.len() == 1 {
        fill_circle(frame, points[0], width * 0.5, color);
        return;
    }
    for pair in points.windows(2) {
        stroke_line(frame, pair[0], pair[1], width, color, true);
    }
    for joint in &points[1..points.len() - 1] {
        fill_circle(frame, *joint, width * 0.5, color);
    }
}

/// 椭圆描边：轮廓线两侧各 `width/2` 的环带。
fn stroke_ellipse(frame: &mut Frame, rect: ERect, width: f32, color: Color32) {
    if !is_positive(width) {
        return;
    }
    let Some((center, rx, ry)) = ellipse_parts(rect) else {
        return;
    };
    let half = width * 0.5;
    let reach = Vec2::new(rx + half + 1.0, ry + half + 1.0);
    let Some(clip) = clip_bounds(frame, center - reach, center + reach) else {
        return;
    };
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let d = ellipse_sdf(Pos2::new(x as f32 + 0.5, py), center, rx, ry);
            blend_pixel(frame, x, y, color, half + 0.5 - d.abs());
        }
    }
}

/// 实心椭圆。
fn fill_ellipse(frame: &mut Frame, rect: ERect, color: Color32) {
    let Some((center, rx, ry)) = ellipse_parts(rect) else {
        return;
    };
    let reach = Vec2::new(rx + 1.0, ry + 1.0);
    let Some(clip) = clip_bounds(frame, center - reach, center + reach) else {
        return;
    };
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let d = ellipse_sdf(Pos2::new(x as f32 + 0.5, py), center, rx, ry);
            blend_pixel(frame, x, y, color, 0.5 - d);
        }
    }
}

/// 矩形描边（`rounding > 0` 时为圆角矩形），线宽以矩形边界为中心。
///
/// 直角情况用「外框覆盖率 - 内框覆盖率」的精确解，比距离场更干净；
/// 圆角情况退回到圆角矩形距离场。
fn stroke_rect(frame: &mut Frame, rect: ERect, width: f32, color: Color32, rounding: f32) {
    if !is_positive(width) {
        return;
    }
    let rect = normalized(rect);
    let half = width * 0.5;
    let Some(clip) = clip_rect(frame, rect.expand(half + 1.0)) else {
        return;
    };
    let rounded = rounding > 0.0;
    let outer = rect.expand(half);
    let inner = inner_rect(rect, half);
    for y in clip.y0..clip.y1 {
        let py = y as f32 + 0.5;
        for x in clip.x0..clip.x1 {
            let px = x as f32 + 0.5;
            let coverage = if rounded {
                let d = rounded_rect_sdf(Pos2::new(px, py), rect, rounding);
                half + 0.5 - d.abs()
            } else {
                box_coverage(outer, px, py) - inner.map_or(0.0, |i| box_coverage(i, px, py))
            };
            blend_pixel(frame, x, y, color, coverage);
        }
    }
}

// ============================================================================
// 文字
// ============================================================================

/// 行高系数：`1.35 × size`，与预览一行的视觉间距一致。
const LINE_HEIGHT: f32 = 1.35;

/// 单个字形位图的像素上限（防御异常大的字号把内存撑爆）。
const MAX_GLYPH_PIXELS: f32 = 16_777_216.0;

/// 按 `origin`（文字**左上角**）排版，返回每个字形的轮廓位图（未绘制）。
///
/// * 基线为 `origin.y + ascent`，与 egui 的 `Align2::LEFT_TOP` 对齐；
/// * 水平推进用 `h_advance`，相邻字符再用 `kern` 修正字偶距；
/// * `\n` 换行并回到 `origin.x`，`\r` 直接忽略；行高为 `size * 1.35`；
/// * 字形位置带小数，因此子像素定位由 `ab_glyph` 的覆盖率负责。
fn layout_glyphs<F: Font>(font: &F, text: &str, size: f32, origin: Pos2) -> Vec<OutlinedGlyph> {
    let mut glyphs = Vec::new();
    if text.is_empty() || !is_positive(size) || !origin.x.is_finite() || !origin.y.is_finite() {
        return glyphs;
    }
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let line_height = size * LINE_HEIGHT;
    let mut pen = ab_glyph::point(origin.x, origin.y + scaled.ascent());
    let mut previous: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        match ch {
            '\r' => continue,
            '\n' => {
                pen.x = origin.x;
                pen.y += line_height;
                previous = None;
                continue;
            }
            _ => {}
        }
        let id = font.glyph_id(ch);
        if let Some(previous) = previous {
            pen.x += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(scale, pen);
        if let Some(outline) = font.outline_glyph(glyph) {
            glyphs.push(outline);
        }
        pen.x += scaled.h_advance(id);
        previous = Some(id);
    }
    glyphs
}

/// 把所有字形位图混合到帧上。
///
/// `ab_glyph` 的回调坐标是相对于 `px_bounds().min`（已向下取整）的整数偏移，
/// 所以这里直接把它当绝对像素下标用，子像素精度由 `layout_glyphs` 决定；
/// 完全在帧外的字形整块跳过。
fn draw_glyphs(frame: &mut Frame, glyphs: &[OutlinedGlyph], color: Color32) {
    let (fw, fh) = (frame.width() as f32, frame.height() as f32);
    for glyph in glyphs {
        let bounds = glyph.px_bounds();
        if bounds.max.x < 0.0 || bounds.max.y < 0.0 || bounds.min.x > fw || bounds.min.y > fh {
            continue;
        }
        let area = (bounds.max.x - bounds.min.x) * (bounds.max.y - bounds.min.y);
        if !is_positive(area) || area > MAX_GLYPH_PIXELS {
            continue;
        }
        let ox = bounds.min.x as i32;
        let oy = bounds.min.y as i32;
        glyph.draw(|gx, gy, coverage| {
            blend_pixel(frame, ox + gx as i32, oy + gy as i32, color, coverage);
        });
    }
}

/// 所有字形像素包围盒的并集（用于把序号数字摆到真正的中心）。
fn glyph_union(glyphs: &[OutlinedGlyph]) -> Option<ERect> {
    let mut iter = glyphs.iter();
    let first = iter.next()?.px_bounds();
    let mut union = ERect::from_min_max(
        Pos2::new(first.min.x, first.min.y),
        Pos2::new(first.max.x, first.max.y),
    );
    for glyph in iter {
        let b = glyph.px_bounds();
        union = union.union(ERect::from_min_max(
            Pos2::new(b.min.x, b.min.y),
            Pos2::new(b.max.x, b.max.y),
        ));
    }
    Some(union)
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cuter_core::geom::Size;
    use egui::Stroke;

    /// 纯黑不透明底图：便于断言「变红 / 仍是黑」。
    fn black(w: u32, h: u32) -> Frame {
        Frame::filled(Size::new(w, h), [0, 0, 0, 255])
    }

    fn is_red(p: [u8; 4]) -> bool {
        p[0] > 200 && p[1] < 60 && p[2] < 60 && p[3] == 255
    }

    /// 粗略探测系统字体（**故意不 import `cuter_ui::fonts`**，
    /// 避免和并行开发的模块产生编译顺序耦合）。找不到就跳过相关测试。
    fn probe_system_font() -> Option<Vec<u8>> {
        const CANDIDATES: [&str; 8] = [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\Deng.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ];
        CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
    }

    #[test]
    fn rect_stroke_paints_the_border_and_keeps_the_inside_black() {
        let base = black(64, 48);
        let annotation = Annotation::Rect {
            rect: ERect::from_min_size(Pos2::new(8.0, 8.0), Vec2::new(40.0, 24.0)),
            stroke: Stroke::new(2.0, Color32::RED),
            fill: None,
            rounding: 0.0,
        };
        let out = flatten(&base, &[annotation], None);

        // 2 像素宽的描边以矩形边界为中心：上下边各覆盖 y = 7、8 两行
        assert!(
            is_red(out.pixel(28, 8)),
            "上边描边应为红色：{:?}",
            out.pixel(28, 8)
        );
        assert!(is_red(out.pixel(28, 7)), "上边描边应为红色（外半像素行）");
        assert!(
            is_red(out.pixel(8, 20)),
            "左边描边应为红色：{:?}",
            out.pixel(8, 20)
        );
        // 内部与外部保持纯黑
        assert_eq!(out.pixel(28, 20), [0, 0, 0, 255], "矩形内部不应被填充");
        assert_eq!(out.pixel(2, 2), [0, 0, 0, 255], "矩形外部不应被改动");
        assert_eq!(out.pixel(60, 44), [0, 0, 0, 255], "右下角外部不应被改动");
    }

    #[test]
    fn diagonal_line_has_anti_aliased_edges() {
        let base = black(16, 16);
        // 1 像素宽的 45° 斜线：线心恰好穿过像素中心，两侧必然落在过渡带里
        let annotation = Annotation::Freehand {
            points: vec![Pos2::new(2.5, 2.5), Pos2::new(13.5, 13.5)],
            stroke: Stroke::new(1.0, Color32::WHITE),
        };
        let out = flatten(&base, &[annotation], None);

        assert_eq!(out.pixel(8, 8), [255, 255, 255, 255], "线心应被完全覆盖");
        let partial = out
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 0 && p[0] < 255)
            .count();
        assert!(partial > 0, "抗锯齿必须产生既非纯前景也非纯背景的中间像素");
    }

    #[test]
    fn ellipse_fill_covers_the_center_but_not_the_corners() {
        let base = black(40, 40);
        let annotation = Annotation::Ellipse {
            rect: ERect::from_min_size(Pos2::new(4.0, 4.0), Vec2::splat(32.0)),
            stroke: Stroke::new(0.0, Color32::TRANSPARENT),
            fill: Some(Color32::from_rgb(0, 0, 255)),
        };
        let out = flatten(&base, &[annotation], None);

        assert_eq!(out.pixel(20, 20), [0, 0, 255, 255], "椭圆中心应被填充");
        // 外接矩形的四个角都在圆之外（若实现退化成矩形填充，这几条会失败）
        assert_eq!(out.pixel(4, 4), [0, 0, 0, 255], "外接矩形左上角在椭圆之外");
        assert_eq!(out.pixel(35, 4), [0, 0, 0, 255], "外接矩形右上角在椭圆之外");
        assert_eq!(out.pixel(4, 35), [0, 0, 0, 255], "外接矩形左下角在椭圆之外");
        assert_eq!(
            out.pixel(35, 35),
            [0, 0, 0, 255],
            "外接矩形右下角在椭圆之外"
        );
        // 上下中点的外侧一点点（椭圆边界在 y = 4 / 36，这里离边界还有 1.5 像素）
        assert_eq!(out.pixel(20, 1), [0, 0, 0, 255], "椭圆上边界之外不应被填充");
        assert_eq!(
            out.pixel(20, 38),
            [0, 0, 0, 255],
            "椭圆下边界之外不应被填充"
        );
    }

    #[test]
    fn missing_font_degrades_instead_of_panicking() {
        let base = black(64, 48);
        let text = Annotation::Text {
            pos: Pos2::new(4.0, 4.0),
            text: "你好 Cuter\n第二行".to_owned(),
            color: Color32::WHITE,
            size: 16.0,
        };
        let counter = Annotation::Counter {
            center: Pos2::new(40.0, 30.0),
            index: 12,
            color: Color32::RED,
            radius: 9.0,
        };
        let out = flatten(&base, &[text, counter], None);

        // 文字降级成占位框：1 像素宽的框线压在半像素上 → 约 50% 覆盖率
        let placeholder = out.pixel(10, 4);
        assert!(
            placeholder[0] > 100,
            "无字体时文字应留下可见的占位框：{placeholder:?}"
        );
        // 序号降级成只画圆（数字不再绘制，但圆心必须是圆色）
        assert!(is_red(out.pixel(40, 30)), "无字体时序号标记仍要画实心圆");
    }

    #[test]
    fn broken_font_bytes_degrade_instead_of_panicking() {
        let base = black(24, 24);
        let bytes = b"definitely not a font".to_vec();
        let text = Annotation::Text {
            pos: Pos2::new(2.0, 2.0),
            text: "hi".to_owned(),
            color: Color32::WHITE,
            size: 12.0,
        };
        let out = flatten(
            &base,
            &[text],
            Some(FontRef {
                bytes: &bytes,
                index: 0,
            }),
        );

        let inked = out.data().as_chunks::<4>().0.iter().any(|p| p[0] > 100);
        assert!(inked, "字体解析失败时应画出占位框而不是 panic");
    }

    #[test]
    fn out_of_bounds_annotations_neither_panic_nor_write() {
        let base = black(24, 24);
        let far_away = vec![
            // 左上角之外（负数）
            Annotation::Rect {
                rect: ERect::from_min_size(Pos2::new(-500.0, -500.0), Vec2::splat(10.0)),
                stroke: Stroke::new(2.0, Color32::RED),
                fill: Some(Color32::RED),
                rounding: 3.0,
            },
            // 右下角之外
            Annotation::Ellipse {
                rect: ERect::from_min_size(Pos2::new(900.0, 900.0), Vec2::splat(10.0)),
                stroke: Stroke::new(2.0, Color32::RED),
                fill: Some(Color32::RED),
            },
            Annotation::Arrow {
                from: Pos2::new(1.0e6, 1.0e6),
                to: Pos2::new(1.0e6 + 10.0, 1.0e6 + 10.0),
                stroke: Stroke::new(2.0, Color32::RED),
                head_len: 8.0,
            },
            Annotation::Freehand {
                points: vec![Pos2::new(-1.0e6, -1.0e6), Pos2::new(-1.0e6 + 1.0, -1.0e6)],
                stroke: Stroke::new(3.0, Color32::RED),
            },
            Annotation::Counter {
                center: Pos2::new(-4000.0, 9000.0),
                index: 1,
                color: Color32::RED,
                radius: 8.0,
            },
            Annotation::Text {
                pos: Pos2::new(-2000.0, -2000.0),
                text: "offscreen".to_owned(),
                color: Color32::RED,
                size: 12.0,
            },
            // 非有限坐标：必须被静默忽略
            Annotation::Text {
                pos: Pos2::new(f32::NAN, f32::NAN),
                text: "nan".to_owned(),
                color: Color32::RED,
                size: 12.0,
            },
            Annotation::Freehand {
                points: vec![Pos2::new(f32::INFINITY, 0.0), Pos2::new(0.0, f32::NAN)],
                stroke: Stroke::new(2.0, Color32::RED),
            },
        ];
        let out = flatten(&base, &far_away, None);
        assert_eq!(
            out.data(),
            base.data(),
            "完全在画布之外的图元不得写进任何像素"
        );

        // 一半在画布外：可见部分要照画，且不能越界
        let straddling = Annotation::Rect {
            rect: ERect::from_min_size(Pos2::new(-6.0, 4.0), Vec2::splat(10.0)),
            stroke: Stroke::new(2.0, Color32::RED),
            fill: None,
            rounding: 0.0,
        };
        let out = flatten(&base, &[straddling], None);
        assert!(is_red(out.pixel(3, 9)), "越界图元的可见部分仍要画出来");
    }

    #[test]
    fn flatten_leaves_the_source_frame_untouched() {
        let base = Frame::filled(Size::new(32, 32), [10, 20, 30, 255]);
        let before = base.data().to_vec();
        let annotations = vec![
            Annotation::Rect {
                rect: ERect::from_min_size(Pos2::new(2.0, 2.0), Vec2::splat(20.0)),
                stroke: Stroke::new(3.0, Color32::RED),
                fill: Some(Color32::from_rgba_unmultiplied(0, 255, 0, 80)),
                rounding: 4.0,
            },
            Annotation::Highlighter {
                points: vec![Pos2::new(1.0, 28.0), Pos2::new(30.0, 20.0)],
                color: Color32::from_rgba_unmultiplied(255, 255, 0, 110),
                width: 8.0,
            },
        ];
        let out = flatten(&base, &annotations, None);

        assert_eq!(base.data(), before.as_slice(), "flatten 不得改动传入的底图");
        assert_eq!(out.size(), base.size(), "副本尺寸必须与原图一致");
        assert_ne!(out.data(), base.data(), "副本上必须真的画上了东西");
    }

    #[test]
    fn source_over_respects_premultiplied_alpha() {
        let mut frame = Frame::new(Size::new(4, 4));
        // egui 的 Color32 是预乘的：这里的分量已经是 128 而不是 255
        let half_white = Color32::from_rgba_unmultiplied(255, 255, 255, 128);
        fill_rect(
            &mut frame,
            ERect::from_min_size(Pos2::ZERO, Vec2::splat(2.0)),
            half_white,
        );

        let p = frame.pixel(0, 0);
        assert!(
            (120..=136).contains(&p[3]),
            "半透明白画到透明底上，alpha 应约为 128：{p:?}"
        );
        assert!(
            p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3],
            "必须满足预乘不变量 rgb <= a（否则 alpha 被算了两遍）：{p:?}"
        );
        assert_eq!(frame.pixel(3, 3), [0, 0, 0, 0], "未覆盖的像素应保持全透明");

        // 不透明底图上的半覆盖：alpha 保持 255，颜色取中间值
        let mut opaque = Frame::filled(Size::new(2, 2), [0, 0, 0, 255]);
        blend_pixel(&mut opaque, 0, 0, Color32::WHITE, 0.5);
        let q = opaque.pixel(0, 0);
        assert_eq!(q[3], 255, "不透明底图的 alpha 不应被降低");
        assert!(
            (q[0] as i32 - 128).abs() <= 1,
            "半覆盖应得到约 50% 灰：{q:?}"
        );

        // 越界写入必须被静默忽略（不 panic、不越界）
        blend_pixel(&mut opaque, -1, -1, Color32::WHITE, 1.0);
        blend_pixel(&mut opaque, 2, 2, Color32::WHITE, 1.0);
        blend_pixel(&mut opaque, 0, 0, Color32::WHITE, f32::NAN);
    }

    #[test]
    fn system_font_renders_text_and_centers_the_counter() {
        let Some(bytes) = probe_system_font() else {
            return; // 找不到字体就跳过，不算失败
        };
        let font = FontRef {
            bytes: &bytes,
            index: 0,
        };
        let base = black(96, 96);

        // 1) 文字：必须真的把字形画进像素
        let text = Annotation::Text {
            pos: Pos2::new(4.0, 4.0),
            text: "Cuter 你好".to_owned(),
            color: Color32::WHITE,
            size: 20.0,
        };
        let text_out = flatten(&base, &[text], Some(font));
        let inked = text_out
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
            .count();
        assert!(
            inked > 50,
            "有字体时必须把字形画进像素，实际着色 {inked} 像素"
        );
        // 字形不会跑出 `pos` 上方太多：说明基线是按 ascent 放的
        let top = (0..96)
            .find(|y| (0..96).any(|x| text_out.pixel(x, *y)[0] > 0))
            .expect("应有着色像素");
        assert!(
            (0..40).contains(&top),
            "文字应从 pos.y 附近开始，实际首个着色行 y = {top}"
        );

        // 2) 序号：数字是白色、圆是红色，白色像素的包围盒中心应贴住圆心
        let counter = Annotation::Counter {
            center: Pos2::new(48.0, 60.0),
            index: 8,
            color: Color32::RED,
            radius: 16.0,
        };
        let counter_out = flatten(&base, &[counter], Some(font));
        assert!(is_red(counter_out.pixel(48, 46)), "序号圆应被填成纯色");
        let mut min = (i32::MAX, i32::MAX);
        let mut max = (i32::MIN, i32::MIN);
        for y in 0..96 {
            for x in 0..96 {
                let p = counter_out.pixel(x, y);
                if p[1] > 100 && p[2] > 100 {
                    min = (min.0.min(x), min.1.min(y));
                    max = (max.0.max(x), max.1.max(y));
                }
            }
        }
        assert!(min.0 <= max.0, "序号数字的白色像素应当存在");
        let cx = (min.0 + max.0) as f32 * 0.5;
        let cy = (min.1 + max.1) as f32 * 0.5;
        assert!(
            (cx - 48.0).abs() <= 2.5 && (cy - 60.0).abs() <= 2.5,
            "序号数字应水平垂直居中，实际中心 ({cx}, {cy})"
        );
    }
}
