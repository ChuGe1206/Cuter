//! 位图容器与常用像素运算。
//!
//! [`Frame`] 是所有捕获结果、裁剪结果与标注结果的统一表示：**紧密排列的 RGBA8**
//! （行主序，无行填充，`len == w * h * 4`）。选择这一布局的原因：
//!
//! * 与 GDI `DIB` 的 `BGRA` 布局只差一次通道交换，捕获路径几乎零拷贝；
//! * 与 egui `ColorImage` 完全一致，上传纹理无需转换；
//! * 与 `image` crate 的 `RgbaImage` 直接互通。

use std::path::Path;

use crate::error::{Error, Result};
use crate::geom::{Point, Rect, Size};

/// RGBA8 像素。
pub type Rgba = [u8; 4];

/// 一张紧密排列的 RGBA8 位图。
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    size: Size,
    data: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("size", &self.size)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl Frame {
    /// 新建全透明位图。
    #[must_use]
    pub fn new(size: Size) -> Self {
        Self {
            size,
            data: vec![0; size.rgba_len()],
        }
    }

    /// 新建单色位图。
    #[must_use]
    pub fn filled(size: Size, color: Rgba) -> Self {
        let mut frame = Self::new(size);
        frame.data = color
            .iter()
            .copied()
            .cycle()
            .take(size.rgba_len())
            .collect();
        frame
    }

    /// 从 RGBA8 缓冲区构造；长度必须精确匹配。
    pub fn from_rgba(size: Size, data: Vec<u8>) -> Result<Self> {
        let expected = size.rgba_len();
        if data.len() != expected {
            return Err(Error::BadBufferSize {
                w: size.w,
                h: size.h,
                expected,
                actual: data.len(),
            });
        }
        Ok(Self { size, data })
    }

    /// 从 GDI / DXGI 常见的 `BGRA`（实际是 `BGRX`）缓冲区构造，第四个通道强制置为
    /// 不透明——捕获到的屏幕没有有效 alpha，若保留 0 会导致整图透明。
    pub fn from_bgrx(size: Size, mut data: Vec<u8>) -> Result<Self> {
        let expected = size.rgba_len();
        if data.len() != expected {
            return Err(Error::BadBufferSize {
                w: size.w,
                h: size.h,
                expected,
                actual: data.len(),
            });
        }
        for px in data.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }
        Ok(Self { size, data })
    }

    #[must_use]
    pub fn size(&self) -> Size {
        self.size
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.size.w
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.size.h
    }

    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.size.w, self.size.h)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// 第 `y` 行的 RGBA 字节切片（`len == w * 4`）。
    #[must_use]
    pub fn row(&self, y: u32) -> &[u8] {
        let w = self.size.w as usize * 4;
        let start = y as usize * w;
        &self.data[start..start + w]
    }

    /// 越界读取返回全透明像素（调用方无需先做边界判断）。
    #[must_use]
    pub fn pixel(&self, x: i32, y: i32) -> Rgba {
        if x < 0 || y < 0 || x >= self.size.w as i32 || y >= self.size.h as i32 {
            return [0, 0, 0, 0];
        }
        let off = (y as usize * self.size.w as usize + x as usize) * 4;
        [
            self.data[off],
            self.data[off + 1],
            self.data[off + 2],
            self.data[off + 3],
        ]
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 || x >= self.size.w as i32 || y >= self.size.h as i32 {
            return;
        }
        let off = (y as usize * self.size.w as usize + x as usize) * 4;
        self.data[off..off + 4].copy_from_slice(&color);
    }

    /// 转置（行列互换）。
    ///
    /// 用途：**横向滚动截屏复用纵向那套拼接代码**——把每一帧转置之后，"横向滚动"在转置
    /// 坐标系里就等价于"纵向滚动"；拼完把结果再转置回来。匹配 / 接缝 / 长度上限那些最容易
    /// 出错的逻辑因此一行都不用改（见 `scroll::DirectedStitcher`）。
    #[must_use]
    pub fn transposed(&self) -> Self {
        let (w, h) = (self.width(), self.height());
        let src = self.data();
        let mut out = vec![0u8; src.len()];
        for y in 0..h {
            for x in 0..w {
                let from = ((y * w + x) * 4) as usize;
                let to = ((x * h + y) * 4) as usize;
                out[to..to + 4].copy_from_slice(&src[from..from + 4]);
            }
        }
        // 尺寸是算出来的（w×h×4 必然等于 len），不会失败
        Self::from_rgba(Size::new(h, w), out).expect("转置后的尺寸与数据长度一致")
    }

    /// 按**局部坐标**裁剪（自动与自身范围求交；完全越界时返回空图）。
    #[must_use]
    pub fn crop(&self, region: Rect) -> Self {
        let region = region.intersect(self.bounds());
        if region.is_empty() {
            return Self::new(Size::ZERO);
        }
        let src_w = self.size.w as usize;
        let row_bytes = region.w as usize * 4;
        let mut out = Vec::with_capacity(region.size().rgba_len());
        for row in 0..region.h as usize {
            let start = ((region.y as usize + row) * src_w + region.x as usize) * 4;
            out.extend_from_slice(&self.data[start..start + row_bytes]);
        }
        Self {
            size: region.size(),
            data: out,
        }
    }

    /// 用纯色填充一块区域（坐标为局部坐标）。
    pub fn fill_rect(&mut self, region: Rect, color: Rgba) {
        let region = region.intersect(self.bounds());
        if region.is_empty() {
            return;
        }
        let row_bytes = region.w as usize * 4;
        let row_pattern: Vec<u8> = color
            .iter()
            .copied()
            .cycle()
            .take(row_bytes)
            .collect();
        for row in 0..region.h as usize {
            let start = ((region.y as usize + row) * self.size.w as usize + region.x as usize) * 4;
            self.data[start..start + row_bytes].copy_from_slice(&row_pattern);
        }
    }

    /// 把 `other` 覆盖到本图 `at` 处（普通 source-over，`other` 视为不透明）。
    pub fn blit(&mut self, other: &Self, at: Point) {
        let dst = Rect::new(at.x, at.y, other.size.w, other.size.h).intersect(self.bounds());
        if dst.is_empty() {
            return;
        }
        let src_x = (dst.x - at.x) as usize;
        let src_y = (dst.y - at.y) as usize;
        let row_bytes = dst.w as usize * 4;
        for row in 0..dst.h as usize {
            let src_start =
                ((src_y + row) * other.size.w as usize + src_x) * 4;
            let dst_start =
                ((dst.y as usize + row) * self.size.w as usize + dst.x as usize) * 4;
            self.data[dst_start..dst_start + row_bytes]
                .copy_from_slice(&other.data[src_start..src_start + row_bytes]);
        }
    }

    /// 把 `other` **按 alpha 合成**到本图 `at` 处（source-over，直通 alpha）。
    ///
    /// 与 [`Frame::blit`] 的区别：这里尊重 `other` 的 alpha 通道，用于叠加带透明边缘
    /// 的素材——最典型的用途是把鼠标光标合成到冻结帧上。
    ///
    /// 公式是非预乘（straight alpha）的标准 source-over：
    ///
    /// ```text
    /// out_a   = sa + da * (1 - sa)
    /// out_rgb = (src_rgb * sa + dst_rgb * da * (1 - sa)) / out_a
    /// ```
    ///
    /// 末尾除以 `out_a` 是必须的：`Frame` 存的是非预乘数据，若照搬预乘公式，
    /// 半透明的光标边缘会比预期亮一圈、出现白边。
    pub fn blend(&mut self, other: &Self, at: Point) {
        let dst = Rect::new(at.x, at.y, other.size.w, other.size.h).intersect(self.bounds());
        if dst.is_empty() {
            return;
        }
        let src_x = (dst.x - at.x) as usize;
        let src_y = (dst.y - at.y) as usize;

        for row in 0..dst.h as usize {
            for col in 0..dst.w as usize {
                let src_off = ((src_y + row) * other.size.w as usize + src_x + col) * 4;
                let dst_off =
                    ((dst.y as usize + row) * self.size.w as usize + dst.x as usize + col) * 4;

                let sa = other.data[src_off + 3] as f32 / 255.0;
                if sa <= 0.0 {
                    continue;
                }
                if sa >= 1.0 {
                    self.data[dst_off..dst_off + 4]
                        .copy_from_slice(&other.data[src_off..src_off + 4]);
                    continue;
                }

                let da = self.data[dst_off + 3] as f32 / 255.0;
                let out_a = sa + da * (1.0 - sa);
                if out_a <= 0.0 {
                    self.data[dst_off..dst_off + 4].copy_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                for channel in 0..3 {
                    let s = other.data[src_off + channel] as f32;
                    let d = self.data[dst_off + channel] as f32;
                    let value = (s * sa + d * da * (1.0 - sa)) / out_a;
                    self.data[dst_off + channel] = value.round().clamp(0.0, 255.0) as u8;
                }
                self.data[dst_off + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    // ------------------------------------------------------------------
    // 标注工具用到的像素效果
    // ------------------------------------------------------------------

    /// 对 `region`（局部坐标）做盒式模糊。`passes` 次迭代可逼近高斯模糊。
    pub fn blur(&mut self, region: Rect, radius: u32, passes: u32) {
        if radius == 0 || passes == 0 || self.is_empty() {
            return;
        }
        let full = self.bounds();
        let region = region.intersect(full);
        if region.is_empty() {
            return;
        }
        let radius = radius.min(full.w.max(full.h)) as usize;
        // 工作区向外扩张 radius，保证边缘取样不出现黑边
        let work = region.inflate(radius as i32).intersect(full);
        let (w, h) = (work.w as usize, work.h as usize);

        let mut buf = vec![0u8; w * h * 4];
        for row in 0..h {
            let src_start =
                ((work.y as usize + row) * full.w as usize + work.x as usize) * 4;
            let dst_start = row * w * 4;
            buf[dst_start..dst_start + w * 4]
                .copy_from_slice(&self.data[src_start..src_start + w * 4]);
        }

        let mut tmp = vec![0u8; buf.len()];
        for _ in 0..passes {
            box_blur_h(&buf, &mut tmp, w, h, radius);
            box_blur_v(&tmp, &mut buf, w, h, radius);
        }

        // 只把 region 部分写回
        let local_x = (region.x - work.x) as usize;
        let local_y = (region.y - work.y) as usize;
        let copy_w = region.w as usize * 4;
        for row in 0..region.h as usize {
            let src_start = ((local_y + row) * w + local_x) * 4;
            let dst_start = ((region.y as usize + row) * full.w as usize + region.x as usize) * 4;
            self.data[dst_start..dst_start + copy_w]
                .copy_from_slice(&buf[src_start..src_start + copy_w]);
        }
    }

    /// 马赛克（像素化）。`block` 为方块边长（像素），最小 2。
    pub fn mosaic(&mut self, region: Rect, block: u32) {
        let region = region.intersect(self.bounds());
        if region.is_empty() {
            return;
        }
        let block = block.max(2) as i32;
        let mut y = region.top();
        while y < region.bottom() {
            let block_bottom = (y + block).min(region.bottom());
            let mut x = region.left();
            while x < region.right() {
                let block_right = (x + block).min(region.right());
                let count = ((block_right - x) * (block_bottom - y)) as u32;
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                for py in y..block_bottom {
                    for px in x..block_right {
                        let p = self.pixel(px, py);
                        r += p[0] as u32;
                        g += p[1] as u32;
                        b += p[2] as u32;
                        a += p[3] as u32;
                    }
                }
                let avg = [
                    (r / count) as u8,
                    (g / count) as u8,
                    (b / count) as u8,
                    (a / count) as u8,
                ];
                for py in y..block_bottom {
                    for px in x..block_right {
                        self.set_pixel(px, py, avg);
                    }
                }
                x = block_right;
            }
            y = block_bottom;
        }
    }

    /// 高质量缩放。
    #[must_use]
    pub fn resized(&self, target: Size, filter: image::imageops::FilterType) -> Self {
        if target.is_empty() || target == self.size {
            return self.clone();
        }
        let src = self.to_rgba_image();
        let dst = image::imageops::resize(&src, target.w, target.h, filter);
        Self::from_rgba(target, dst.into_raw())
            .expect("resize 输出尺寸恒等于请求尺寸")
    }

    /// 按比例缩放（用于「截图美化」里的分辨率导出）。
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        let target = Size::new(
            ((self.size.w as f32 * factor).round() as u32).max(1),
            ((self.size.h as f32 * factor).round() as u32).max(1),
        );
        self.resized(target, image::imageops::FilterType::Lanczos3)
    }

    // ------------------------------------------------------------------
    // 与 image / egui 互转 + 落盘
    // ------------------------------------------------------------------

    /// 转换为 `image::RgbaImage`（零拷贝语义上的必要拷贝，尺寸恒匹配）。
    #[must_use]
    pub fn to_rgba_image(&self) -> image::RgbaImage {
        image::RgbaImage::from_raw(self.size.w, self.size.h, self.data.clone())
            .expect("Frame 不变量：data.len() == w * h * 4")
    }

    #[must_use]
    pub fn from_rgba_image(img: image::RgbaImage) -> Self {
        let size = Size::new(img.width(), img.height());
        Self {
            size,
            data: img.into_raw(),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let img = image::load_from_memory(bytes)
            .map_err(|_| Error::UnknownImageFormat)?
            .to_rgba8();
        Ok(Self::from_rgba_image(img))
    }

    /// 编码为指定格式的字节流。
    pub fn encode(&self, format: ImageFormat) -> Result<Vec<u8>> {
        let dyn_img = image::DynamicImage::ImageRgba8(self.to_rgba_image());
        let mut out = Vec::new();
        match format {
            ImageFormat::Png => dyn_img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?,
            ImageFormat::Bmp => dyn_img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Bmp)?,
            ImageFormat::Jpeg { quality } => {
                let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    std::io::Cursor::new(&mut out),
                    quality.clamp(1, 100),
                );
                dyn_img.to_rgb8().write_with_encoder(encoder)?;
            }
            ImageFormat::WebP => {
                let encoder = image::codecs::webp::WebPEncoder::new_lossless(std::io::Cursor::new(&mut out));
                dyn_img.write_with_encoder(encoder)?;
            }
        }
        Ok(out)
    }

    /// 按扩展名保存；扩展名缺失时回退到 PNG。
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let format = ImageFormat::from_path(path).unwrap_or(ImageFormat::Png);
        self.save_as(path, format)
    }

    /// 按**指定格式**保存，不看扩展名。
    ///
    /// 为什么需要它：格式来自设置（`SaveConfig::image_format()`，已经带上用户配置的
    /// JPEG 质量），而 [`Frame::save`] 只能从扩展名反推一个内置默认质量 92 ——
    /// 走那条路，「JPEG 质量」这个设置永远传不到编码器。
    pub fn save_as(&self, path: impl AsRef<Path>, format: ImageFormat) -> Result<()> {
        let path = path.as_ref();
        let bytes = self.encode(format)?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(path, bytes).map_err(|e| Error::io(path, e))?;
        Ok(())
    }
}

/// 支持导出的图片格式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg { quality: u8 },
    WebP,
    Bmp,
}

impl ImageFormat {
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg { quality: 92 }),
            "webp" => Some(Self::WebP),
            "bmp" => Some(Self::Bmp),
            _ => None,
        }
    }

    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        Self::from_extension(path.extension()?.to_str()?)
    }

    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg { .. } => "jpg",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
        }
    }

    /// 用于设置界面下拉框的用户可见名称。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG（无损，推荐）",
            Self::Jpeg { .. } => "JPEG（体积小）",
            Self::WebP => "WebP（无损、体积更小）",
            Self::Bmp => "BMP（兼容性最好）",
        }
    }

    /// 这个格式是否吃「质量」参数（设置界面据此决定要不要显示滑块）。
    #[must_use]
    pub fn is_lossy(self) -> bool {
        matches!(self, Self::Jpeg { .. })
    }

    /// 覆盖有损格式的质量；无损格式原样返回。
    ///
    /// 为什么需要它：`from_extension` / `from_path` 只看得到扩展名，拿不到用户的
    /// 配置，只能填一个内置默认质量。配置层的出口必须再覆盖一次，否则
    /// `save.quality` 就是一个「拖了没反应」的假开关。
    #[must_use]
    pub fn with_quality(self, quality: u8) -> Self {
        match self {
            Self::Jpeg { .. } => Self::Jpeg {
                quality: quality.clamp(1, 100),
            },
            other => other,
        }
    }
}

impl std::str::FromStr for ImageFormat {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::from_extension(s).ok_or_else(|| Error::UnsupportedFormat(s.to_owned()))
    }
}

/// 水平方向滑动窗口盒式模糊（边缘复制）。
fn box_blur_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let window = (2 * r + 1) as u32;
    let last = w as isize - 1;
    for y in 0..h {
        let row = y * w * 4;
        for c in 0..4 {
            let mut sum: u32 = 0;
            for i in -(r as isize)..=(r as isize) {
                sum += src[row + i.clamp(0, last) as usize * 4 + c] as u32;
            }
            for x in 0..w {
                dst[row + x * 4 + c] = (sum / window) as u8;
                let leaving = (x as isize - r as isize).clamp(0, last) as usize;
                let entering = (x as isize + r as isize + 1).clamp(0, last) as usize;
                sum = sum + src[row + entering * 4 + c] as u32 - src[row + leaving * 4 + c] as u32;
            }
        }
    }
}

/// 垂直方向滑动窗口盒式模糊（边缘复制）。
fn box_blur_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let window = (2 * r + 1) as u32;
    let stride = w * 4;
    let last = h as isize - 1;
    for x in 0..w {
        let col = x * 4;
        for c in 0..4 {
            let mut sum: u32 = 0;
            for i in -(r as isize)..=(r as isize) {
                sum += src[i.clamp(0, last) as usize * stride + col + c] as u32;
            }
            for y in 0..h {
                dst[y * stride + col + c] = (sum / window) as u8;
                let leaving = (y as isize - r as isize).clamp(0, last) as usize;
                let entering = (y as isize + r as isize + 1).clamp(0, last) as usize;
                sum = sum + src[entering * stride + col + c] as u32
                    - src[leaving * stride + col + c] as u32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        let mut f = Frame::new(Size::new(4, 3));
        for y in 0..3 {
            for x in 0..4 {
                let v = (y * 4 + x) as u8;
                f.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        f
    }

    #[test]
    fn crop_uses_local_coords_and_clamps() {
        let f = sample();
        let c = f.crop(Rect::new(1, 1, 2, 2));
        assert_eq!(c.size(), Size::new(2, 2));
        assert_eq!(c.pixel(0, 0), [5, 5, 5, 255]);
        assert_eq!(c.pixel(1, 1), [10, 10, 10, 255]);

        let clamped = f.crop(Rect::new(3, 2, 10, 10));
        assert_eq!(clamped.size(), Size::new(1, 1));
        assert_eq!(clamped.pixel(0, 0), [11, 11, 11, 255]);

        assert!(f.crop(Rect::new(50, 50, 2, 2)).is_empty());
    }

    #[test]
    fn bgrx_swaps_channels_and_forces_opaque() {
        let f = Frame::from_bgrx(Size::new(2, 1), vec![1, 2, 3, 0, 4, 5, 6, 99]).unwrap();
        assert_eq!(f.pixel(0, 0), [3, 2, 1, 255]);
        assert_eq!(f.pixel(1, 0), [6, 5, 4, 255]);
    }

    #[test]
    fn from_rgba_rejects_wrong_length() {
        assert!(Frame::from_rgba(Size::new(2, 2), vec![0; 15]).is_err());
    }

    #[test]
    fn blur_spreads_a_hard_edge() {
        let mut f = Frame::filled(Size::new(16, 4), [0, 0, 0, 255]);
        f.fill_rect(Rect::new(8, 0, 8, 4), [255, 255, 255, 255]);
        f.blur(Rect::new(0, 0, 16, 4), 3, 1);
        // 边界处应出现中间灰度
        let mid = f.pixel(7, 2)[0];
        assert!(mid > 0 && mid < 255, "期望中间灰度，实际 {mid}");
        // 远离边界处应保持原样
        assert_eq!(f.pixel(0, 2)[0], 0);
        assert_eq!(f.pixel(15, 2)[0], 255);
    }

    #[test]
    fn mosaic_uses_block_average() {
        let mut f = Frame::new(Size::new(4, 4));
        f.fill_rect(Rect::new(0, 0, 2, 4), [0, 0, 0, 255]);
        f.fill_rect(Rect::new(2, 0, 2, 4), [100, 100, 100, 255]);
        f.mosaic(Rect::new(0, 0, 4, 4), 4);
        let avg = f.pixel(0, 0)[0];
        assert_eq!(avg, 50);
        assert_eq!(f.pixel(3, 3)[0], avg);
    }

    #[test]
    fn png_roundtrip_preserves_pixels() {
        let f = sample();
        let bytes = f.encode(ImageFormat::Png).unwrap();
        let back = Frame::decode(&bytes).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn jpeg_encode_drops_alpha_without_panicking() {
        let bytes = sample().encode(ImageFormat::Jpeg { quality: 90 }).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(Frame::decode(&bytes).unwrap().size(), Size::new(4, 3));
    }

    #[test]
    fn with_quality_only_touches_lossy_formats() {
        assert!(matches!(ImageFormat::Png.with_quality(10), ImageFormat::Png));
        assert!(matches!(ImageFormat::WebP.with_quality(10), ImageFormat::WebP));
        assert!(matches!(ImageFormat::Bmp.with_quality(10), ImageFormat::Bmp));

        assert!(matches!(
            ImageFormat::Jpeg { quality: 92 }.with_quality(37),
            ImageFormat::Jpeg { quality: 37 }
        ));
        // 越界值夹取到编码器接受的区间，而不是原样传下去
        assert!(matches!(
            ImageFormat::Jpeg { quality: 92 }.with_quality(0),
            ImageFormat::Jpeg { quality: 1 }
        ));
        assert!(matches!(
            ImageFormat::Jpeg { quality: 92 }.with_quality(250),
            ImageFormat::Jpeg { quality: 100 }
        ));
        assert!(ImageFormat::Jpeg { quality: 92 }.is_lossy());
        assert!(!ImageFormat::Png.is_lossy());
    }

    /// 「质量设置是假开关」的回归测试：**真的编码一遍**，低质量必须更小。
    /// 只断言 `ImageFormat` 的字段相等是不够的——字段对了但没传进编码器，
    /// 用户看到的仍然是「拖了滑块体积没变」。
    #[test]
    fn jpeg_quality_really_reaches_the_encoder() {
        let mut f = Frame::new(Size::new(96, 96));
        for y in 0..96 {
            for x in 0..96 {
                f.set_pixel(
                    x,
                    y,
                    [
                        (x * 2) as u8,
                        (y * 2) as u8,
                        ((x ^ y) * 2) as u8,
                        255,
                    ],
                );
            }
        }
        let high = f
            .encode(ImageFormat::Jpeg { quality: 95 })
            .unwrap()
            .len();
        let low = f.encode(ImageFormat::Jpeg { quality: 20 }).unwrap().len();
        assert!(
            low < high,
            "低质量应当更小：q20={low} 字节，q95={high} 字节"
        );
    }

    /// `save_as` 必须真的按传入的格式写盘，而不是回头去看扩展名。
    ///
    /// 这条守的是「设置里的 JPEG 质量能到磁盘上」这条链路：
    /// 文件名是 `x.png` 也要能按要求存成 JPEG（`Frame::save` 做不到）。
    #[test]
    fn save_as_follows_the_given_format_not_the_extension() {
        let dir = std::env::temp_dir().join("cuter-frame-save-as");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quality-probe.png");
        let frame = sample();

        frame
            .save_as(&path, ImageFormat::Jpeg { quality: 20 })
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // JPEG 的 SOI 标记；PNG 会是 89 50 4E 47
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "扩展名是 .png 也必须按 JPEG 编码");

        frame.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47], "save 仍然按扩展名走");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resize_preserves_requested_size() {
        let f = sample().resized(Size::new(8, 6), image::imageops::FilterType::Nearest);
        assert_eq!(f.size(), Size::new(8, 6));
    }

    #[test]
    fn blit_clips_to_destination() {
        let mut dst = Frame::filled(Size::new(4, 4), [0, 0, 0, 255]);
        let src = Frame::filled(Size::new(3, 3), [9, 9, 9, 255]);
        dst.blit(&src, Point::new(3, 3));
        assert_eq!(dst.pixel(3, 3), [9, 9, 9, 255]);
        assert_eq!(dst.pixel(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn blend_opaque_source_overwrites() {
        let mut dst = Frame::filled(Size::new(2, 2), [0, 0, 0, 255]);
        let src = Frame::filled(Size::new(2, 2), [200, 100, 50, 255]);
        dst.blend(&src, Point::ZERO);
        assert_eq!(dst.pixel(0, 0), [200, 100, 50, 255]);
    }

    #[test]
    fn blend_transparent_source_is_a_noop() {
        let mut dst = Frame::filled(Size::new(2, 2), [10, 20, 30, 255]);
        let src = Frame::filled(Size::new(2, 2), [255, 255, 255, 0]);
        dst.blend(&src, Point::ZERO);
        assert_eq!(dst.pixel(0, 0), [10, 20, 30, 255]);
    }

    #[test]
    fn blend_half_alpha_averages_in_straight_alpha_space() {
        // 半透明白叠在不透明黑上：期望恰好是中灰，而不是预乘公式给出的偏亮值
        let mut dst = Frame::filled(Size::new(1, 1), [0, 0, 0, 255]);
        let src = Frame::filled(Size::new(1, 1), [255, 255, 255, 128]);
        dst.blend(&src, Point::ZERO);
        let out = dst.pixel(0, 0);
        assert_eq!(out[3], 255, "不透明底图之上合成后仍应不透明");
        for channel in 0..3 {
            assert!(
                (out[channel] as i32 - 128).abs() <= 2,
                "直线 alpha 合成应得到约 128，实际 {out:?}"
            );
        }
    }

    #[test]
    fn blend_onto_transparent_background_keeps_source_color() {
        // 半透明红叠到全透明上：结果应是同样的红色，alpha 变成源的 alpha
        let mut dst = Frame::new(Size::new(1, 1));
        let src = Frame::filled(Size::new(1, 1), [255, 0, 0, 128]);
        dst.blend(&src, Point::ZERO);
        let out = dst.pixel(0, 0);
        assert_eq!((out[0], out[1], out[2]), (255, 0, 0), "不能被背景拉黑");
        assert!((out[3] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn blend_clips_like_blit() {
        let mut dst = Frame::filled(Size::new(4, 4), [0, 0, 0, 255]);
        let src = Frame::filled(Size::new(3, 3), [255, 255, 255, 255]);
        // 部分越界不应 panic，且只影响相交区域
        dst.blend(&src, Point::new(-1, -1));
        assert_eq!(dst.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(dst.pixel(3, 3), [0, 0, 0, 255]);
        dst.blend(&src, Point::new(10, 10));
    }
}

/// 把四角按圆角裁掉（只改 alpha，颜色不动）。
///
/// 用途：贴图窗口的"图片本身是圆角"（`pin.rounded_corners`）。阴影早就是圆角了，
/// 图片本身还是直角，两者放一起会露出来。
///
/// 边缘做了一点抗锯齿：圆角半径内的像素按"覆盖率"给 alpha，免得出现锯齿台阶。
/// `radius` 会被夹到不超过短边的一半；`radius == 0` 时原样返回（零开销快路径）。
#[must_use]
pub fn rounded_corners(frame: &Frame, radius: u32) -> Frame {
    let (w, h) = (frame.width(), frame.height());
    let radius = radius.min(w.min(h) / 2);
    if radius == 0 || w == 0 || h == 0 {
        return frame.clone();
    }
    let mut out = frame.clone();
    let r = radius as f32;
    let data = out.data_mut();
    for y in 0..h {
        for x in 0..w {
            // 该像素属于哪个圆心（不在四个角上就直接跳过）
            let cx = if x < radius {
                r
            } else if x >= w - radius {
                (w - radius) as f32
            } else {
                continue;
            };
            let cy = if y < radius {
                r
            } else if y >= h - radius {
                (h - radius) as f32
            } else {
                continue;
            };
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            // 1 像素宽的过渡带：distance <= r-0.5 全保留，>= r+0.5 全透明
            let coverage = ((r + 0.5 - distance) / 1.0).clamp(0.0, 1.0);
            if coverage >= 1.0 {
                continue;
            }
            let offset = ((y * w + x) * 4 + 3) as usize;
            let alpha = f32::from(data[offset]);
            data[offset] = (alpha * coverage).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod rounded_tests {
    use super::*;
    use crate::geom::Size;

    fn solid(w: u32, h: u32) -> Frame {
        Frame::filled(Size::new(w, h), [10, 20, 30, 255])
    }

    fn alpha(frame: &Frame, x: u32, y: u32) -> u8 {
        frame.data()[((y * frame.width() + x) * 4 + 3) as usize]
    }

    #[test]
    fn four_corners_become_transparent_and_center_stays_opaque() {
        let frame = solid(20, 20);
        let rounded = rounded_corners(&frame, 6);
        for (x, y) in [(0, 0), (19, 0), (0, 19), (19, 19)] {
            assert_eq!(alpha(&rounded, x, y), 0, "角落 ({x},{y}) 应该全透明");
        }
        assert_eq!(alpha(&rounded, 10, 10), 255, "中心不该被动");
        // 颜色不动，只改 alpha
        assert_eq!(rounded.data()[0], 10);
        // 边缘上"整像素覆盖率"的位置应当完全保留（比如左边中点）
        assert_eq!(alpha(&rounded, 0, 10), 255);
    }

    #[test]
    fn radius_zero_and_oversized_radius_are_handled() {
        let frame = solid(12, 8);
        let same = rounded_corners(&frame, 0);
        assert_eq!(same.data(), frame.data(), "radius=0 应当原样返回");
        // 半径超过短边一半：夹到一半，不 panic、中心仍然保留
        let clamped = rounded_corners(&frame, 99);
        assert_eq!(alpha(&clamped, 6, 4), 255);
        assert_eq!(alpha(&clamped, 0, 0), 0);
    }

    #[test]
    fn anti_aliasing_keeps_partial_alpha_somewhere_on_the_curve() {
        let frame = solid(40, 40);
        let rounded = rounded_corners(&frame, 10);
        let partial = (0..40)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let a = alpha(&rounded, *x, *y);
                a > 0 && a < 255
            })
            .count();
        assert!(partial > 0, "圆角边缘应当有半透明像素（抗锯齿）");
    }
}