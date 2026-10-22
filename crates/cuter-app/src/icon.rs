//! 应用图标（运行时用代码画出来，不携带任何图片资源）。
//!
//! 为什么要自己画：
//!
//! * 托盘图标必须是 RGBA 原始数据，`tray-icon` 不接受 SVG；
//! * 打包一个 `.ico`/`.png` 会引入「资源文件路径」这类跨平台麻烦；
//! * 32×32 的图标用不到 100 行代码就能画得干净，还顺带避免了二进制资源进仓库。
//!
//! 图形语义：「甘」形取景框（圆角方框 + 四角刻度 + 中心焦点）。**两套皮肤**：
//!
//! * `Purple`（曜夜紫，配深色主题）：底色 `#2A1650 → #0B0618`，线条 `#8CF8FF`；
//! * `Mint`（薄荷奶白，配浅色主题）：底色 `#F6FCF9 → #D6EFE6`，线条 `#14A88E`。
//!
//! 两套必须成对存在：深紫底放在亮任务栏上是一块脏斑，奶白底放在深色任务栏上糊成一团。
//! 辉光用同色系半透明描边叠两圈实现，不用滤镜——缩到 16px 与导出 `.ico` 都不会走样
//! （浅色皮肤下这两圈是**柔影**，浅底上"发光"看不出层次）。
//!
//! 几何全部按 `docs/design/icons/viewfinder-badge.svg`（256 网格）等比换算。

use std::sync::atomic::{AtomicU8, Ordering};

/// 托盘图标尺寸。
pub const TRAY_SIZE: u32 = 32;
/// 窗口图标尺寸。
pub const WINDOW_SIZE: u32 = 64;

/// 图标皮肤：跟界面主题一起换。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSkin {
    /// 曜夜紫——深色主题。
    Purple,
    /// 薄荷奶白——浅色主题。
    Mint,
}

/// 一套图标配色。
struct Palette {
    bg_top: [u8; 3],
    bg_bottom: [u8; 3],
    glow: [u8; 3],
    glow_alpha: [f32; 2],
    line: [u8; 3],
    dot: [u8; 3],
}

impl IconSkin {
    fn palette(self) -> Palette {
        match self {
            Self::Purple => Palette {
                bg_top: [0x2A, 0x16, 0x50],
                bg_bottom: [0x0B, 0x06, 0x18],
                glow: [0x7D, 0xF9, 0xFF],
                glow_alpha: [0.10, 0.18],
                line: [0x8C, 0xF8, 0xFF],
                dot: [0xFF, 0xFF, 0xFF],
            },
            Self::Mint => Palette {
                bg_top: [0xF6, 0xFC, 0xF9],
                bg_bottom: [0xD6, 0xEF, 0xE6],
                glow: [0x0C, 0x8A, 0x78],
                glow_alpha: [0.14, 0.22],
                line: [0x14, 0xA8, 0x8E],
                dot: [0x0B, 0x5F, 0x52],
            },
        }
    }

    /// 皮肤代号（写日志用，纯 ASCII）。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Purple => "purple",
            Self::Mint => "mint",
        }
    }
}

/// 当前皮肤。托盘、任务栏、窗口图标三处分别取用，用一个静态量省得处处传参。
static CURRENT_SKIN: AtomicU8 = AtomicU8::new(0);

/// 设置当前皮肤（主题解析出来后调用）。
pub fn set_skin(skin: IconSkin) {
    let code = match skin {
        IconSkin::Purple => 0,
        IconSkin::Mint => 1,
    };
    CURRENT_SKIN.store(code, Ordering::Relaxed);
}

/// 读取当前皮肤。
#[must_use]
pub fn skin() -> IconSkin {
    match CURRENT_SKIN.load(Ordering::Relaxed) {
        1 => IconSkin::Mint,
        _ => IconSkin::Purple,
    }
}

/// 托盘图标的 RGBA 数据（32×32，当前皮肤）。
#[must_use]
pub fn tray_icon_rgba() -> Vec<u8> {
    render_for(skin(), TRAY_SIZE)
}

/// 指定皮肤的托盘图标。
#[must_use]
pub fn tray_icon_rgba_for(skin: IconSkin) -> Vec<u8> {
    render_for(skin, TRAY_SIZE)
}

/// 窗口图标（当前皮肤）。
#[must_use]
pub fn window_icon() -> egui::IconData {
    window_icon_for(skin())
}

/// 指定皮肤的窗口图标。
#[must_use]
pub fn window_icon_for(skin: IconSkin) -> egui::IconData {
    let size = WINDOW_SIZE;
    egui::IconData {
        rgba: render_for(skin, size),
        width: size,
        height: size,
    }
}

/// 当前皮肤的窗口图标（`Arc` 版，给 `ViewportCommand::Icon` 用）。
#[must_use]
pub fn window_icon_shared() -> Option<std::sync::Arc<egui::IconData>> {
    Some(std::sync::Arc::new(window_icon()))
}

/// 把指定皮肤的图标画进一块 RGBA 缓冲区。
///
/// 所有几何都以稿子的 256 网格为基准，按 `size / 256` 换算到像素。
#[must_use]
pub fn render_for(skin: IconSkin, size: u32) -> Vec<u8> {
    let palette = skin.palette();
    let s = size as f32;
    let k = s / 256.0;
    let mut buffer = vec![0u8; (size * size * 4) as usize];

    let badge_radius = 58.0 * k;
    // 辉光：(内缩, 边长, 圆角, 描边宽)
    let glow: [(f32, f32, f32, f32); 2] = [
        (58.0 * k, 140.0 * k, 30.0 * k, 30.0 * k),
        (66.0 * k, 124.0 * k, 24.0 * k, 20.0 * k),
    ];
    // 方框：(内缩, 边长, 圆角, 描边宽)
    let frame = (76.0 * k, 104.0 * k, 18.0 * k, 15.0 * k);
    let tick_width = 13.0 * k;
    let ticks: [((f32, f32), (f32, f32)); 4] = [
        ((76.0 * k, 44.0 * k), (76.0 * k, 64.0 * k)),
        ((44.0 * k, 76.0 * k), (64.0 * k, 76.0 * k)),
        ((180.0 * k, 44.0 * k), (180.0 * k, 64.0 * k)),
        ((212.0 * k, 76.0 * k), (192.0 * k, 76.0 * k)),
    ];
    let dot_radius = 14.0 * k;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            // 圆角方块底色
            let background = rounded_rect_coverage(px, py, s, badge_radius);
            if background <= 0.0 {
                continue;
            }
            // 从上到下的渐变
            let t = (py / s).clamp(0.0, 1.0);
            let mut color = mix(palette.bg_top, palette.bg_bottom, t);

            // 辉光 / 柔影：两圈半透明描边
            for (index, (inset, side, radius, width)) in glow.into_iter().enumerate() {
                let coverage = rounded_rect_outline_coverage(px, py, inset, side, radius, width);
                let alpha = palette.glow_alpha[index];
                color = mix(color, palette.glow, (coverage * alpha).clamp(0.0, 1.0));
            }

            // 「甘」形取景框
            let (inset, side, radius, width) = frame;
            let mut line = rounded_rect_outline_coverage(px, py, inset, side, radius, width);
            for (a, b) in ticks {
                let d = distance_to_segment(px, py, a, b);
                line = line.max((tick_width * 0.5 + 0.5 - d).clamp(0.0, 1.0));
            }
            color = mix(color, palette.line, line);

            // 中心焦点
            let d = ((px - s * 0.5).powi(2) + (py - s * 0.5).powi(2)).sqrt();
            let dot = (dot_radius + 0.5 - d).clamp(0.0, 1.0);
            color = mix(color, palette.dot, dot);

            let alpha = (background * 255.0).round().clamp(0.0, 255.0) as u8;
            let offset = ((y * size + x) * 4) as usize;
            buffer[offset] = color[0];
            buffer[offset + 1] = color[1];
            buffer[offset + 2] = color[2];
            buffer[offset + 3] = alpha;
        }
    }
    buffer
}

/// 圆角矩形的有符号距离（左上角在原点、边长 `size`、圆角 `radius`）。
fn rounded_rect_distance(x: f32, y: f32, size: f32, radius: f32) -> f32 {
    let half = size * 0.5;
    let qx = (x - half).abs() - (half - radius);
    let qy = (y - half).abs() - (half - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

/// 圆角矩形描边的抗锯齿覆盖率（`[0, 1]`）。
fn rounded_rect_outline_coverage(x: f32, y: f32, inset: f32, size: f32, radius: f32, width: f32) -> f32 {
    let d = rounded_rect_distance(x - inset, y - inset, size, radius);
    (width * 0.5 + 0.5 - d.abs()).clamp(0.0, 1.0)
}

/// 圆角矩形的抗锯齿覆盖率（`[0, 1]`）。
fn rounded_rect_coverage(x: f32, y: f32, size: f32, radius: f32) -> f32 {
    (0.5 - rounded_rect_distance(x, y, size, radius)).clamp(0.0, 1.0)
}

/// 点到线段的距离。
fn distance_to_segment(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ax, ay) = a;
    let (bx, by) = b;
    let dx = bx - ax;
    let dy = by - ay;
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq <= f32::EPSILON {
        0.0
    } else {
        (((x - ax) * dx + (y - ay) * dy) / length_sq).clamp(0.0, 1.0)
    };
    let px = ax + dx * t;
    let py = ay + dy * t;
    ((x - px).powi(2) + (y - py).powi(2)).sqrt()
}

/// 线性插值两个 RGB 颜色。
fn mix(from: [u8; 3], to: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for i in 0..3 {
        out[i] = (from[i] as f32 + (to[i] as f32 - from[i] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_produces_expected_buffer_size() {
        assert_eq!(
            render_for(IconSkin::Purple, 16).len(),
            16 * 16 * 4
        );
        assert_eq!(tray_icon_rgba().len(), (TRAY_SIZE * TRAY_SIZE * 4) as usize);
    }

    #[test]
    fn corners_are_transparent_and_center_is_opaque() {
        let size = TRAY_SIZE;
        let rgba = render_for(IconSkin::Purple, size);
        let at = |x: u32, y: u32| {
            let o = ((y * size + x) * 4) as usize;
            [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
        };
        assert_eq!(at(0, 0)[3], 0, "圆角之外必须完全透明");
        assert_eq!(at(size - 1, size - 1)[3], 0);
        assert!(at(size / 2, 2)[3] > 200, "顶部中间应当是不透明的底色");
    }

    #[test]
    fn purple_skin_matches_the_viewfinder_design() {
        let size = TRAY_SIZE;
        let rgba = render_for(IconSkin::Purple, size);
        let at = |x: u32, y: u32| {
            let o = ((y * size + x) * 4) as usize;
            [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
        };
        // 底色是很深的紫：整体很暗，且蓝通道明显高于红通道
        let top = at(size / 2, 3);
        assert!(top[0] < 90 && top[1] < 90 && top[2] < 140, "底色应为深紫：{top:?}");
        assert!(top[2] > top[0] + 20, "底色应当偏紫蓝：{top:?}");
        // 中心是白色焦点
        let center = at(size / 2, size / 2);
        assert!(
            center[0] > 200 && center[1] > 200 && center[2] > 200,
            "中心应为白色焦点：{center:?}"
        );
        // 方框描边是霓虹青：在左边缘附近扫一遍取最亮的一列
        let mut best = [0u8; 4];
        for x in 5..14 {
            let p = at(x, size / 2);
            if p[1] as u32 + p[2] as u32 > best[1] as u32 + best[2] as u32 {
                best = p;
            }
        }
        assert!(best[1] > 150 && best[2] > 180, "方框描边应为霓虹青：{best:?}");
    }

    /// 浅色皮肤：奶白底 + 薄荷青框 + 深薄荷焦点。
    ///
    /// 这一套是给亮任务栏 / 浅色主题用的，判据和深色皮肤相反——底色必须**很亮**，
    /// 框的绿通道必须明显高于红通道，中心点必须是深色（白点在奶白底上会消失）。
    #[test]
    fn mint_skin_is_milk_white_with_mint_lines() {
        let size = TRAY_SIZE;
        let rgba = render_for(IconSkin::Mint, size);
        let at = |x: u32, y: u32| {
            let o = ((y * size + x) * 4) as usize;
            [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
        };
        let top = at(size / 2, 3);
        assert!(
            top[0] > 200 && top[1] > 200 && top[2] > 200,
            "底色应为奶白：{top:?}"
        );
        let center = at(size / 2, size / 2);
        assert!(
            i32::from(center[1]) > i32::from(center[0]) + 30 && center[1] < 160,
            "中心应为深薄荷（不是白点）：{center:?}"
        );
        // 浅底上"最亮"的是背景，判据得换成"最绿"——取绿通道减去红通道最大的一列。
        let mut best = [0u8; 4];
        let mut best_score = i32::MIN;
        for x in 5..14 {
            let p = at(x, size / 2);
            let score = i32::from(p[1]) - i32::from(p[0]);
            if score > best_score {
                best_score = score;
                best = p;
            }
        }
        assert!(
            best[1] > 120 && i32::from(best[1]) > i32::from(best[0]) + 40,
            "方框描边应为薄荷青：{best:?}"
        );
    }

    /// 全局皮肤开关：托盘与窗口图标都走它。
    #[test]
    fn skin_switch_is_global() {
        set_skin(IconSkin::Mint);
        assert_eq!(skin(), IconSkin::Mint);
        assert_eq!(tray_icon_rgba(), tray_icon_rgba_for(IconSkin::Mint));
        assert_eq!(window_icon().rgba, window_icon_for(IconSkin::Mint).rgba);
        set_skin(IconSkin::Purple);
        assert_eq!(skin(), IconSkin::Purple);
        assert_eq!(tray_icon_rgba(), tray_icon_rgba_for(IconSkin::Purple));
    }

    #[test]
    fn window_icon_matches_requested_size() {
        assert_eq!(window_icon().rgba.len(), (WINDOW_SIZE * WINDOW_SIZE * 4) as usize);
    }

    #[test]
    fn distance_to_segment_is_correct_for_endpoints_and_middle() {
        let a = (0.0, 0.0);
        let b = (10.0, 0.0);
        assert!((distance_to_segment(0.0, 0.0, a, b) - 0.0).abs() < 1e-4);
        assert!((distance_to_segment(5.0, 3.0, a, b) - 3.0).abs() < 1e-4);
        assert!((distance_to_segment(-4.0, 0.0, a, b) - 4.0).abs() < 1e-4);
        assert!((distance_to_segment(14.0, 0.0, a, b) - 4.0).abs() < 1e-4);
    }

    #[test]
    fn degenerate_segment_does_not_divide_by_zero() {
        let d = distance_to_segment(3.0, 4.0, (0.0, 0.0), (0.0, 0.0));
        assert!((d - 5.0).abs() < 1e-4);
    }
}
