//! 冻结帧毛玻璃：浮动面板底下垫的那层「压暗 + 模糊 + 提亮」底图。
//!
//! 设计稿（Figma）里的做法是：面板矩形取冻结帧像素，做两遍盒式模糊，再叠 10% 白，
//! 全程只用我们自己的像素数据，**不依赖系统合成器**（Windows 上没有可靠的跨版本毛玻璃）。
//!
//! 这里更进一步：先整帧缩到 1/4 再模糊——面板本身会把它放大回去，多出来的那点插值
//! 正是「奶玻璃」的观感，而代价只有几十毫秒一次（冻结帧在整个遮罩会话里不变，算一次缓存住）。

use cuter_core::frame::Frame;

/// 把冻结帧做成毛玻璃底图。
///
/// * `factor` —— 先缩放的比例（0.25 够用；`1.0` 表示不缩放）
/// * `radius` / `passes` —— 在缩略图上做盒式模糊（设计稿是两遍）
/// * `white` —— 叠白比例（设计稿 10%）
/// * `dim` —— 压暗比例，和遮罩盖在桌面上的那层保持一致，免得面板底比周围亮一块
#[must_use]
pub fn frost(base: &Frame, factor: f32, radius: u32, passes: u32, white: f32, dim: f32) -> Frame {
    let mut out = if (factor - 1.0).abs() < f32::EPSILON {
        base.clone()
    } else {
        base.scaled(factor)
    };

    if radius > 0 && passes > 0 && !out.size().is_empty() {
        out.blur(out.bounds(), radius, passes);
    }

    let white = white.clamp(0.0, 1.0);
    let dim = dim.clamp(0.0, 1.0);
    if white > 0.0 || dim < 1.0 {
        for px in out.data_mut().as_chunks_mut::<4>().0 {
            for channel in &mut px[..3] {
                let mut value = f32::from(*channel) * dim;
                value += (255.0 - value) * white;
                *channel = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cuter_core::geom::Size;

    /// 棋盘格：相邻像素黑白交替，高频细节拉满。
    fn checkerboard(size: u32) -> Frame {
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let value = if (x + y) % 2 == 0 { 0_u8 } else { 255_u8 };
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }
        Frame::from_rgba(Size::new(size, size), data).expect("棋盘格是合法位图")
    }

    fn solid(size: u32, value: u8) -> Frame {
        Frame::from_rgba(Size::new(size, size), [value, value, value, 255].repeat((size * size) as usize))
            .expect("纯色图是合法位图")
    }

    /// 红通道的均值与方差。
    fn mean_and_variance(frame: &Frame) -> (f32, f32) {
        let pixels: Vec<f32> = frame
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| f32::from(px[0]))
            .collect();
        let count = pixels.len() as f32;
        let mean = pixels.iter().sum::<f32>() / count;
        let variance = pixels
            .iter()
            .map(|value| (value - mean) * (value - mean))
            .sum::<f32>()
            / count;
        (mean, variance)
    }

    #[test]
    fn blur_kills_high_frequency_detail() {
        let base = checkerboard(128);
        let (_, before) = mean_and_variance(&base);
        let out = frost(&base, 0.25, 6, 2, 0.0, 1.0);
        let (_, after) = mean_and_variance(&out);
        assert!(
            after < before * 0.05,
            "模糊后高频方差应该基本消失：{before} → {after}"
        );
    }

    #[test]
    fn white_overlay_brightens_by_the_requested_ratio() {
        let out = frost(&solid(32, 0), 1.0, 0, 0, 0.10, 1.0);
        let (mean, _) = mean_and_variance(&out);
        // 黑底叠 10% 白 ≈ 25.5，且不该被裁到纯白
        assert!((mean - 25.5).abs() < 1.0, "叠白 10% 的均值应约 25.5，实际 {mean}");
        assert!(mean < 250.0, "叠白不该把面板底刷成纯白");
    }

    #[test]
    fn dim_darkens_by_the_requested_ratio() {
        let out = frost(&solid(32, 255), 1.0, 0, 0, 0.0, 0.5);
        let (mean, _) = mean_and_variance(&out);
        assert!((mean - 127.5).abs() < 1.0, "压暗 50% 的均值应约 127.5，实际 {mean}");
    }
}
