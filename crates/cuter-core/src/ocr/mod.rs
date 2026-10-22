//! OCR 抽象层。
//!
//! 「框选后立刻识别」是这类工具最核心的体验，所以这里从一开始就把接口固定下来：
//! [`OcrEngine`] trait + [`OcrResult`] / [`TextBlock`] 数据结构，UI 只依赖它们。
//!
//! ## 引擎选择
//!
//! | `config.ocr.engine` | 实现 | 代价 |
//! |---|---|---|
//! | `Disabled` | 无 | —— |
//! | `Builtin` | [`windows_ocr::WindowsOcr`]（`Windows.Media.Ocr`） | 零下载零打包，完全离线 |
//! | `Http` | [`http_ocr::HttpOcr`]（POST 给自建 PaddleOCR 服务） | 需要额外服务 |
//!
//! 之所以把「内置」定成**系统自带**的 OCR，而不是打包 PP-OCR 模型：本项目的硬约束
//! 是存储占用要小，而 PP-OCR 的模型加运行时是几十 MB 级别，`Windows.Media.Ocr`
//! 则是系统已经装在机器上的能力。需要更高精度时再走 `Http`。

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::geom::Rect;

pub mod http_ocr;
pub mod json;

#[cfg(windows)]
pub mod windows_ocr;

pub use http_ocr::HttpOcr;

#[cfg(windows)]
pub use windows_ocr::WindowsOcr;

/// 一个被识别出来的文本块（坐标为**输入图片的局部物理像素**）。
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    /// 四边形包围盒（支持旋转文本，顺序为左上→右上→右下→左下）。
    pub quad: [crate::geom::Point; 4],
    /// 轴对齐包围盒，方便直接拿来做高亮绘制。
    pub bounds: Rect,
    pub text: String,
    /// 置信度 0.0-1.0。
    pub score: f32,
}

impl TextBlock {
    /// 轴对齐包围盒由四边形推导。
    #[must_use]
    pub fn from_quad(quad: [crate::geom::Point; 4], text: String, score: f32) -> Self {
        let xs = quad.map(|p| p.x);
        let ys = quad.map(|p| p.y);
        let bounds = Rect::from_ltrb(
            *xs.iter().min().unwrap_or(&0),
            *ys.iter().min().unwrap_or(&0),
            *xs.iter().max().unwrap_or(&0),
            *ys.iter().max().unwrap_or(&0),
        );
        Self {
            quad,
            bounds,
            text,
            score,
        }
    }

    /// 四个角是否正好落在轴对齐包围盒的四个角上。
    ///
    /// 系统引擎（`WindowsOcr`）只会给这种矩形；在线引擎可能给真正的旋转四边形，
    /// 那时高亮就得画多边形而不是圆角矩形。
    #[must_use]
    pub fn is_axis_aligned(&self) -> bool {
        let corners = [
            (self.bounds.x, self.bounds.y),
            (self.bounds.right(), self.bounds.y),
            (self.bounds.right(), self.bounds.bottom()),
            (self.bounds.x, self.bounds.bottom()),
        ];
        corners
            .iter()
            .all(|(x, y)| self.quad.iter().any(|p| p.x == *x && p.y == *y))
    }

    /// 四边形面积（鞋带公式）。重叠时用它挑最小的块——轴对齐时结果与 `w * h` 相同。
    #[must_use]
    pub fn area(&self) -> u64 {
        let mut twice = 0i64;
        for i in 0..4 {
            let a = self.quad[i];
            let b = self.quad[(i + 1) % 4];
            twice += i64::from(a.x) * i64::from(b.y) - i64::from(b.x) * i64::from(a.y);
        }
        (twice.abs() / 2) as u64
    }

    /// 点是否落在这个四边形里（局部物理像素）。
    ///
    /// 先用轴对齐包围盒挡一道，保持"左闭右开"的边界语义与旧行为一致；再用四个叉积
    /// 判断点是否在凸四边形内。旋转/竖排的框必须走后者，否则斜框外面的"空白角"
    /// 也会被算成命中（用户指着框外却高亮了那一行）。
    #[must_use]
    pub fn contains(&self, point: crate::geom::Point) -> bool {
        if !self.bounds.contains(point) {
            return false;
        }
        let mut sign = 0i32;
        for i in 0..4 {
            let a = self.quad[i];
            let b = self.quad[(i + 1) % 4];
            let cross = i64::from(b.x - a.x) * i64::from(point.y - a.y)
                - i64::from(b.y - a.y) * i64::from(point.x - a.x);
            if cross == 0 {
                continue;
            }
            let this = if cross > 0 { 1 } else { -1 };
            if sign == 0 {
                sign = this;
            } else if sign != this {
                return false;
            }
        }
        // 四个叉积全为 0：点与每条边共线，只有退化四边形才会这样
        true
    }

    /// 编号徽标该挂的角：最上面那个，同高取最左。
    #[must_use]
    pub fn top_left_corner(&self) -> crate::geom::Point {
        let mut best = self.quad[0];
        for p in &self.quad[1..] {
            if p.y < best.y || (p.y == best.y && p.x < best.x) {
                best = *p;
            }
        }
        best
    }
}

/// 一次识别的完整结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcrResult {
    pub blocks: Vec<TextBlock>,
    /// 引擎耗时（毫秒），用于诊断。
    pub elapsed_ms: u64,
}

impl OcrResult {
    /// 纯文本（`paragraph = true` 时按行拼接）。
    #[must_use]
    pub fn plain_text(&self, paragraph: bool) -> String {
        let mut lines: Vec<&str> = self
            .blocks
            .iter()
            .map(|b| b.text.trim())
            .filter(|t| !t.is_empty())
            .collect();
        if paragraph {
            // 简单分段：以标点结尾的行视为段落结束
            let mut out = String::new();
            for line in lines.drain(..) {
                out.push_str(line);
                if line.ends_with(['。', '！', '？', '.', '!', '?', '；', ';']) {
                    out.push('\n');
                }
            }
            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            out
        } else {
            lines.join("\n")
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// 图片上这个点落在哪个文本块里（局部物理像素）。
    ///
    /// 重叠时返回**面积最小**的那个：OCR 引擎偶尔会把一行里的一小段也单独报成块，
    /// 用户指着那一小段时想要的是那一小段，不是包含它的大块。
    #[must_use]
    pub fn block_at(&self, point: crate::geom::Point) -> Option<usize> {
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.contains(point))
            .min_by_key(|(_, block)| block.area())
            .map(|(index, _)| index)
    }

    /// 按"人读的顺序"重排文本块，目前只处理一种情况：**整页都是竖排**。
    ///
    /// 竖排中文/日文是"从右到左的列，每列从上到下"，而引擎给的是它自己检测出来的顺序
    /// （PaddleOCR 尤其如此），读起来是乱的。只有**每个块都是竖排**时才按列重排；
    /// 横竖混排的页面保持引擎原序——真实混排样本还没验过，宁可不排也不乱排。
    #[must_use]
    pub fn in_reading_order(mut self) -> Self {
        if let Some(order) = vertical_columns(&self.blocks) {
            let mut blocks = Vec::with_capacity(order.len());
            for index in order {
                blocks.push(self.blocks[index].clone());
            }
            self.blocks = blocks;
        }
        self
    }
}

/// 这个框里的字是竖着排的吗：看**长边**是竖直的还是水平的。
///
/// 竖排的一列字是"又高又窄"的框，长边竖直；横排的一行字长边水平。不能看
/// `quad[0] → quad[1]`：引擎给四个角的顺序是"沿着文字方向"走的，但对一个本来就
/// 又高又窄的框，那条边是框的顶边，判不出文字方向。
///
/// 长边要**明显**更竖直（两倍以上）才算竖排：单个字的框接近正方形，
/// 按长短边判定会随机倒向一边，那种块不该把整页的阅读顺序翻过来。
fn is_vertical(block: &TextBlock) -> bool {
    let a = block.quad[0];
    let b = block.quad[1];
    let c = block.quad[2];
    let first = (i64::from(b.x - a.x), i64::from(b.y - a.y));
    let second = (i64::from(c.x - b.x), i64::from(c.y - b.y));
    let (dx, dy) = if second.0.abs() + second.1.abs() > first.0.abs() + first.1.abs() {
        second
    } else {
        first
    };
    dy.abs() > dx.abs() * 2
}

/// 竖排重排的顺序：从右到左的列，列内从上到下。`None` = 这一页不适用。
fn vertical_columns(blocks: &[TextBlock]) -> Option<Vec<usize>> {
    if blocks.len() < 2 || !blocks.iter().all(is_vertical) {
        return None;
    }

    // 先按水平中线从右到左扫；x 范围有重叠的归同一列
    let mut by_x: Vec<usize> = (0..blocks.len()).collect();
    by_x.sort_by_key(|&index| -i64::from(blocks[index].bounds.center().x));
    let mut columns: Vec<(i32, i32, Vec<usize>)> = Vec::new();
    for index in by_x {
        let bounds = blocks[index].bounds;
        match columns.last_mut() {
            Some((left, right, members))
                if bounds.x.max(*left) < bounds.right().min(*right) =>
            {
                *left = (*left).min(bounds.x);
                *right = (*right).max(bounds.right());
                members.push(index);
            }
            _ => columns.push((bounds.x, bounds.right(), vec![index])),
        }
    }

    let mut order = Vec::with_capacity(blocks.len());
    for (_, _, mut members) in columns {
        members.sort_by_key(|&index| blocks[index].bounds.center().y);
        order.extend(members);
    }
    Some(order)
}

/// OCR 引擎接口。
pub trait OcrEngine: Send + Sync {
    /// 引擎名称（显示在识别面板标题）。
    fn name(&self) -> &str;

    /// 识别一张图片。
    fn recognize(&self, image: &Frame) -> Result<OcrResult>;

    /// 引擎是否可用（模型是否已下载等）。
    fn is_ready(&self) -> bool {
        true
    }
}

/// 占位实现：任何调用都返回「未启用」。
///
/// 它的存在让 UI 与 CLI 可以在没有模型的情况下完整跑通流程。
#[derive(Debug, Default, Clone, Copy)]
pub struct DisabledEngine;

impl OcrEngine for DisabledEngine {
    fn name(&self) -> &str {
        "未启用"
    }

    fn recognize(&self, _image: &Frame) -> Result<OcrResult> {
        Err(Error::Ocr("OCR 引擎未启用（设置 → OCR）".into()))
    }

    fn is_ready(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    #[test]
    fn from_quad_derives_axis_aligned_bounds() {
        let block = TextBlock::from_quad(
            [
                Point::new(10, 20),
                Point::new(60, 18),
                Point::new(62, 40),
                Point::new(12, 42),
            ],
            "你好".into(),
            0.98,
        );
        assert_eq!(block.bounds, Rect::new(10, 18, 52, 24));
    }

    /// 轴对齐的框：面积就是 w×h，徽标角就是左上角。
    #[test]
    fn an_axis_aligned_quad_reports_its_area_and_corner() {
        let block = TextBlock::from_quad(
            [
                Point::new(10, 20),
                Point::new(60, 20),
                Point::new(60, 40),
                Point::new(10, 40),
            ],
            "正的".to_owned(),
            1.0,
        );
        assert!(block.is_axis_aligned());
        assert_eq!(block.area(), 50 * 20);
        assert_eq!(block.top_left_corner(), Point::new(10, 20));
    }

    /// 旋转的框：包围盒照旧，但命中测试只认真正的四边形，
    /// 否则斜框外面那四个"空白角"也会被算成命中（用户指着框外却高亮了那一行）。
    #[test]
    fn a_rotated_quad_is_hit_only_inside_the_polygon() {
        // 45° 菱形：包围盒 0,0 - 100×100，顶点在四条边的中点上
        let block = TextBlock::from_quad(
            [
                Point::new(50, 0),
                Point::new(100, 50),
                Point::new(50, 100),
                Point::new(0, 50),
            ],
            "斜着的字".to_owned(),
            0.9,
        );
        assert_eq!(block.bounds, Rect::new(0, 0, 100, 100));
        assert!(!block.is_axis_aligned(), "斜框不该被当成轴对齐");
        assert_eq!(block.area(), 5000, "菱形面积是包围盒的一半");
        assert_eq!(block.top_left_corner(), Point::new(50, 0), "徽标挂在最上的角");
        assert!(block.contains(Point::new(50, 50)), "中间必须在");
        assert!(!block.contains(Point::new(5, 5)), "左上角在包围盒里、在四边形外");
        assert!(!block.contains(Point::new(95, 95)), "右下角同理");
    }

    /// 旋转框与正框重叠：斜框包围盒里的"空白角"不该抢走命中。
    #[test]
    fn a_rotated_box_does_not_steal_a_hit_from_its_bounding_corner() {
        let rotated = TextBlock::from_quad(
            [
                Point::new(50, 0),
                Point::new(100, 50),
                Point::new(50, 100),
                Point::new(0, 50),
            ],
            "斜的".to_owned(),
            0.9,
        );
        let small = TextBlock::from_quad(
            [
                Point::new(2, 2),
                Point::new(40, 2),
                Point::new(40, 40),
                Point::new(2, 40),
            ],
            "正的".to_owned(),
            0.9,
        );
        let result = OcrResult {
            blocks: vec![rotated, small],
            elapsed_ms: 1,
        };
        assert_eq!(
            result.block_at(Point::new(20, 20)),
            Some(1),
            "包围盒左上角那块只有小框真的覆盖"
        );
        assert_eq!(result.block_at(Point::new(50, 50)), Some(0), "菱形中间是大框");
    }

    #[test]
    fn plain_text_paragraph_mode_breaks_on_punctuation() {        let result = OcrResult {
            blocks: vec![
                TextBlock::from_quad(
                    [Point::ZERO; 4],
                    "第一句话。".into(),
                    1.0,
                ),
                TextBlock::from_quad([Point::ZERO; 4], "第二句".into(), 1.0),
            ],
            elapsed_ms: 3,
        };
        assert_eq!(result.plain_text(true), "第一句话。\n第二句\n");
        assert_eq!(result.plain_text(false), "第一句话。\n第二句");
    }

    #[test]
    fn disabled_engine_reports_error_and_not_ready() {
        let engine = DisabledEngine;
        assert!(!engine.is_ready());
        assert!(engine.recognize(&Frame::new(crate::geom::Size::new(1, 1))).is_err());
    }

    /// 命中测试：重叠时给**最小**的那个（用户指的是那一小段，不是包着它的大块）。
    #[test]
    fn block_at_prefers_the_smallest_containing_block() {
        use crate::geom::{Point, Rect};
        let big = TextBlock::from_quad(
            [
                Point::new(0, 0),
                Point::new(200, 0),
                Point::new(200, 40),
                Point::new(0, 40),
            ],
            "整行".to_owned(),
            0.9,
        );
        let small = TextBlock::from_quad(
            [
                Point::new(10, 5),
                Point::new(60, 5),
                Point::new(60, 35),
                Point::new(10, 35),
            ],
            "一小段".to_owned(),
            0.9,
        );
        let result = OcrResult {
            blocks: vec![big, small],
            elapsed_ms: 1,
        };
        assert_eq!(result.block_at(Point::new(30, 20)), Some(1), "重叠处该给最小的");
        assert_eq!(result.block_at(Point::new(150, 20)), Some(0), "只落在大块里");
        assert_eq!(result.block_at(Point::new(300, 20)), None, "谁都不含这个点");
        // 边界是左闭右开（与 `Rect::contains` 一致）
        assert_eq!(result.block_at(Point::new(200, 20)), None);
        let _ = Rect::new(0, 0, 1, 1);
    }

    /// 一个"又高又窄"的竖排块：40 × 120 的框（长边 120 竖直，够两倍判定）。
    fn vertical_box(text: &str, x: i32, y: i32) -> TextBlock {
        use crate::geom::Point;
        TextBlock::from_quad(
            [
                Point::new(x, y),
                Point::new(x + 40, y),
                Point::new(x + 40, y + 120),
                Point::new(x, y + 120),
            ],
            text.to_owned(),
            0.9,
        )
    }

    fn texts_of(result: &OcrResult) -> Vec<&str> {
        result.blocks.iter().map(|block| block.text.as_str()).collect()
    }

    /// 整页竖排：从右到左的列，列内从上到下；引擎给的左→右乱序要被纠正。
    #[test]
    fn vertical_columns_are_read_right_to_left() {
        let result = OcrResult {
            blocks: vec![
                vertical_box("左一", 180, 60),
                vertical_box("右一", 420, 60),
                vertical_box("中一", 300, 60),
                vertical_box("左二", 180, 200),
                vertical_box("右二", 420, 200),
                vertical_box("中二", 300, 200),
            ],
            elapsed_ms: 1,
        }
        .in_reading_order();
        assert_eq!(
            texts_of(&result),
            ["右一", "右二", "中一", "中二", "左一", "左二"]
        );
    }

    /// 只有一列时也要按 y 排（引擎可能先给中间那块）。
    #[test]
    fn one_vertical_column_reads_top_to_bottom() {
        let result = OcrResult {
            blocks: vec![
                vertical_box("三", 100, 400),
                vertical_box("一", 100, 60),
                vertical_box("二", 100, 230),
            ],
            elapsed_ms: 1,
        }
        .in_reading_order();
        assert_eq!(texts_of(&result), ["一", "二", "三"]);
    }

    /// 横竖混排：不动。真实混排样本没验过，宁可不排也不乱排。
    #[test]
    fn a_mixed_page_keeps_the_engine_order() {
        use crate::geom::Point;
        let horizontal = TextBlock::from_quad(
            [
                Point::new(100, 500),
                Point::new(400, 500),
                Point::new(400, 540),
                Point::new(100, 540),
            ],
            "横着的一行".to_owned(),
            0.9,
        );
        let result = OcrResult {
            blocks: vec![vertical_box("左", 180, 60), horizontal, vertical_box("右", 420, 60)],
            elapsed_ms: 1,
        }
        .in_reading_order();
        assert_eq!(texts_of(&result), ["左", "横着的一行", "右"]);
    }

    /// 单个字的框接近正方形，长边不够两倍竖直，不算竖排——否则整页顺序会被它翻过来。
    #[test]
    fn a_square_block_is_not_vertical() {
        use crate::geom::Point;
        let square = TextBlock::from_quad(
            [
                Point::new(100, 100),
                Point::new(140, 100),
                Point::new(140, 140),
                Point::new(100, 140),
            ],
            "单".to_owned(),
            0.9,
        );
        let result = OcrResult {
            blocks: vec![vertical_box("右", 420, 60), square],
            elapsed_ms: 1,
        }
        .in_reading_order();
        assert_eq!(texts_of(&result), ["右", "单"]);
    }

    /// 重排之后，面板文本、复制出来的纯文本都跟着走。
    #[test]
    fn the_reading_order_reaches_the_plain_text() {
        let result = OcrResult {
            blocks: vec![vertical_box("左", 180, 60), vertical_box("右", 420, 60)],
            elapsed_ms: 1,
        }
        .in_reading_order();
        assert_eq!(result.plain_text(false), "右\n左");
    }
}
