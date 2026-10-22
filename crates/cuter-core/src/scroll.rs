//! 滚动截屏：从连续帧里求出滚动位移，并把它们拼成一张长图。
//!
//! ## 为什么把算法单独放一层
//!
//! 滚动截屏的难点全在「两帧之间到底滚了多少像素」这一件事上：抓帧、拼图都是机械操作，
//! 而位移估计要面对重复内容、粘性页头、噪声与误匹配。把它做成**不依赖任何平台 API 的
//! 纯函数**之后，就能用合成图把每种失败情形都钉死在单测里——否则你只能靠"滚一下看看
//! 拼得对不对"，那是最不可靠的验证方式。
//!
//! ## 算法
//!
//! 1. **先在行签名上找位移**。为每行算一个短签名（横向均匀取若干个采样列的亮度），
//!    然后把「上一个窗口」与「当前窗口」的签名做滑动比对，取平均绝对误差最小的位移。
//!    先用签名是为了把 O(宽 × 高 × 位移数) 降到 O(高 × 位移数 × 采样数)。
//! 2. **排除粘性页头**。网页的固定导航栏在滚动时不动，如果把它算进匹配，它会稀释
//!    误差差异。所以先找出两帧开头完全相同的行数 `s`，匹配只在 `[s, h)` 上做。
//! 3. **周期性内容选保守的一侧**。列表里出现等高的重复图案时，`p、2p、…` 都会匹配得
//!    一样好——取大了会把已出现过的内容再拼一遍（长图里同一段文字出现两次，很显眼），
//!    取小了则会漏掉一段。两者都错，这里选**较小**的位移，并在文档里如实写明这个限制。
//!    真实内容极少出现完美周期，所以这条 tie-break 很少触发。
//!
//! ## 坐标系
//!
//! 所有 [`Frame`] 都是同一个屏幕区域的连续截图，尺寸必须一致。
//! 位移 `d` 的含义：**内容向上移动了 `d` 像素**（也就是用户向下滚动），
//! 于是当前帧底部新露出的 `d` 行就是需要追加到长图尾部的部分。

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::geom::Size;

/// 位移估计的参数。
#[derive(Clone, Copy, Debug)]
pub struct MatchOptions {
    /// 最大搜索位移。越大越耗时间（复杂度与它成正比）。
    pub max_offset: u32,
    /// 小于该位移视为"没有滚动"。
    pub min_offset: u32,
    /// 每行取多少个采样列来构造签名。
    pub sample_columns: usize,
    /// 每几行取一行参与比较（抽稀以提速）。
    pub row_step: u32,
    /// 平均绝对误差上限；超过就认为没匹配上（比如内容整个变了）。
    pub max_mean_error: f32,
    /// 判定"粘性"时允许的行差异（0 表示要求完全相同）。
    pub sticky_tolerance: u8,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            max_offset: 1200,
            min_offset: 2,
            sample_columns: 24,
            row_step: 2,
            // 屏幕内容经过了缩放/抗锯齿时不会逐字节相同，留一点余量
            max_mean_error: 12.0,
            sticky_tolerance: 2,
        }
    }
}

/// 一次成功的位移估计。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchResult {
    /// 内容向上移动的像素数。
    pub offset: u32,
    /// 匹配区域的平均绝对误差（越小越可信）。
    pub mean_error: f32,
    /// 判定为粘性（不随滚动移动）的顶部行数。
    pub sticky_rows: u32,
}

/// 一行的签名：在整幅宽度上均匀取 `samples` 个像素的亮度。
fn row_signature(frame: &Frame, y: u32, samples: usize, out: &mut [u8]) {
    let width = frame.width();
    if width == 0 || samples == 0 {
        return;
    }
    let row = frame.row(y);
    for (index, slot) in out.iter_mut().enumerate() {
        // 均匀分布，避免总是取到最左边那几个像素
        let x = (index * width as usize / samples).min(width as usize - 1);
        let px = &row[x * 4..x * 4 + 4];
        // 亮度：整数近似（0.299R + 0.587G + 0.114B）
        *slot = ((px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) >> 8) as u8;
    }
}

/// 为**每一行**预计算签名，下标就是行号。
///
/// 早期版本为了提速把行也抽稀了（签名数组下标 = `y / row_step`），那是错的：
/// 位移在比较时被量化到 `row_step` 的整数倍，于是**奇数位移根本表达不出来**，
/// 落在同一个桶里的位移给出完全相同的误差，最终只能靠 tie-break 猜——实测表现为
/// 「真实滚了 60 像素，拼出来却按 61 拼」。现在行签名保持全分辨率（1080 行 × 24 字节
/// 只有 26 KB），抽稀改为在**比较循环内部**按 `row_step` 跳行，
/// 这样每个候选位移都有自己独立的比较样本集。
fn signatures(frame: &Frame, samples: usize) -> Vec<Vec<u8>> {
    let rows = frame.height();
    let mut out = Vec::with_capacity(rows as usize);
    for y in 0..rows {
        let mut signature = vec![0u8; samples];
        row_signature(frame, y, samples, &mut signature);
        out.push(signature);
    }
    out
}

/// 两行是否「足够相同」（逐字节差异都不超过 `tolerance`）。
fn rows_match(frame_a: &Frame, y_a: u32, frame_b: &Frame, y_b: u32, tolerance: u8) -> bool {
    let width = frame_a.width().min(frame_b.width());
    if width == 0 {
        return true;
    }
    let row_a = frame_a.row(y_a);
    let row_b = frame_b.row(y_b);
    // 抽样比较即可：整行逐字节在 4K 宽度下太慢，而抽样已经足够判定"同一行"
    let step = (width as usize / 64).max(1) * 4;
    let mut index = 0;
    while index + 3 < width as usize * 4 {
        for channel in 0..4 {
            let a = row_a[index + channel];
            let b = row_b[index + channel];
            if a.abs_diff(b) > tolerance {
                return false;
            }
        }
        index += step;
    }
    true
}

/// 两帧开头**完全相同**的行数（用于识别粘性页头）。
///
/// 只有两帧尺寸一致时才有意义；尺寸不同返回 0。
#[must_use]
pub fn sticky_prefix_rows(prev: &Frame, next: &Frame, tolerance: u8) -> u32 {
    if prev.size() != next.size() {
        return 0;
    }
    let height = prev.height();
    let mut count = 0;
    while count < height && rows_match(prev, count, next, count, tolerance) {
        count += 1;
    }
    count
}

/// 求两帧之间的滚动位移。
///
/// 返回 `None` 表示**没找到可信的位移**（内容整个变了、或者根本没滚）。
pub fn find_vertical_offset(
    prev: &Frame,
    next: &Frame,
    options: &MatchOptions,
) -> Option<MatchResult> {
    if prev.size() != next.size() || prev.is_empty() {
        return None;
    }
    let height = prev.height();
    let sticky = sticky_prefix_rows(prev, next, options.sticky_tolerance);

    // 匹配只在粘性带以下进行
    let usable = height.saturating_sub(sticky);
    if usable < 8 {
        return None;
    }

    // 至少要保留这么多重叠行，否则比较样本太少、结论不可信
    let min_overlap = (usable / 4).max(8);
    let max_offset = options
        .max_offset
        .min(usable.saturating_sub(min_overlap))
        .max(options.min_offset);
    if max_offset < options.min_offset {
        return None;
    }

    let previous = signatures(prev, options.sample_columns);
    let current = signatures(next, options.sample_columns);
    let step = options.row_step.max(1);

    let mut best: Option<(u32, f32)> = None;
    for offset in options.min_offset..=max_offset {
        // 比较 prev 的 [sticky+offset, height) 与 next 的 [sticky, height-offset)。
        // 抽稀只发生在比较循环内部，候选位移本身保持全分辨率。
        let mut sum: u64 = 0;
        let mut count: u64 = 0;
        let mut row = sticky;
        while row + offset < height {
            let a = &previous[(row + offset) as usize];
            let b = &current[row as usize];
            for (x, y) in a.iter().zip(b.iter()) {
                sum += x.abs_diff(*y) as u64;
            }
            count += a.len() as u64;
            row += step;
        }
        if count == 0 {
            continue;
        }
        let mean = sum as f32 / count as f32;
        if mean > options.max_mean_error {
            continue;
        }
        match best {
            None => best = Some((offset, mean)),
            Some((_, best_error)) => {
                // 误差相近时**取较小的位移**。
                //
                // 内容出现周期性图案（例如等高的列表项）时，`p、2p、3p…` 会给出几乎相同的
                // 误差，此时两种选择都不完美：取大了会把已经出现过的内容再拼一遍
                // （长图里同一段文字出现两次，非常显眼），取小了则会漏掉一段。
                // 两者都错，但**重复**更容易被用户一眼看出是坏图，所以选保守的那一侧。
                // 真实内容里几乎不会出现这种完美的周期，因此这条 tie-break 极少触发。
                if mean < best_error * 0.5 {
                    best = Some((offset, mean));
                }
            }
        }
    }

    best.map(|(offset, mean_error)| MatchResult {
        offset,
        mean_error,
        sticky_rows: sticky,
    })
}

/// 一次 `push` 的结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PushOutcome {
    /// 拼接成功，在**末尾**新增了 `offset` 行（用户往下滚）。
    Appended { offset: u32 },
    /// 拼接成功，在**开头**新增了 `offset` 行（用户往上滚，即"反向滚动"）。
    Prepended { offset: u32 },
    /// 没有检测到滚动（内容与上一帧相同）。
    Unchanged,
    /// 没匹配上：内容变化太大，无法确定位移。
    NoMatch,
    /// 已经达到长度上限，不再追加。
    TooTall,
}

/// 把连续帧累积成一张长图。
///
/// 内部用一条不断增长的 RGBA 字节流保存结果，追加是 `extend_from_slice`（均摊 O(1)），
/// 因此不会出现"每拼一帧就把整张长图复制一遍"的平方级开销。
pub struct ScrollStitcher {
    width: u32,
    /// 行主序的 RGBA 数据，长度恒为 `width * height * 4`。
    data: Vec<u8>,
    height: u32,
    max_height: u32,
    /// 上一帧，用于与下一帧做匹配。
    last: Frame,
    frames: u32,
    /// 最近一次匹配结果，供 UI 显示。
    last_match: Option<MatchResult>,
    /// 画布在**当前帧上方**还留着多少行（前插累积出来的）。
    ///
    /// 这两个"余量"是反向滚动能正确工作的关键：用户往回滚到**已经拍过的内容**时，
    /// 不能傻乎乎地再前插一遍（那会长图里出现重复段落）。于是：
    /// 前插量 = `offset - head_room`，追加量 = `offset - tail_room`，两侧余量随窗口移动而增减。
    head_room: u32,
    /// 画布在**当前帧下方**还留着多少行（追加累积出来的）。
    tail_room: u32,
}

impl std::fmt::Debug for ScrollStitcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollStitcher")
            .field("size", &(self.width, self.height))
            .field("frames", &self.frames)
            .field("max_height", &self.max_height)
            .finish()
    }
}

impl ScrollStitcher {
    /// 以第一帧为长图的起点。
    ///
    /// `max_height` 是长图的长度上限——不设上限的话，一个滚不到底的页面能把内存吃光
    /// （且 GL 的纹理边长上限通常是 16384，再高连预览都传不上去）。
    #[must_use]
    pub fn new(first: Frame, max_height: u32) -> Self {
        let width = first.width();
        let height = first.height().min(max_height);
        let data = first.data()[..(width * height * 4) as usize].to_vec();
        Self {
            width,
            data,
            height,
            max_height: max_height.max(1),
            last: first,
            frames: 1,
            last_match: None,
            head_room: 0,
            tail_room: 0,
        }
    }

    /// 长图当前的尺寸。
    #[must_use]
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// 已经吃进去多少帧。
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// 最近一次匹配的细节。
    #[must_use]
    pub fn last_match(&self) -> Option<MatchResult> {
        self.last_match
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.height >= self.max_height
    }

    /// 取出长图（会产生一次完整拷贝，供导出/预览使用）。
    #[must_use]
    pub fn image(&self) -> Frame {
        Frame::from_rgba(Size::new(self.width, self.height), self.data.clone())
            .expect("ScrollStitcher 不变量：data.len() == width * height * 4")
    }

    /// 生成一张缩略图，**不拷贝原图**。
    ///
    /// 长图可以长到上万行，每次刷新都 `image()` 一次就是几十 MB 的拷贝；
    /// 而预览只需要一张能塞进窗口的小图。这里按步长抽样直接读内部缓冲，
    /// 开销只与**输出**尺寸有关。
    #[must_use]
    pub fn thumbnail(&self, max: Size) -> Frame {
        let source_w = self.width.max(1);
        let source_h = self.height.max(1);
        let max_w = max.w.max(1);
        let max_h = max.h.max(1);

        let scale = (max_w as f32 / source_w as f32)
            .min(max_h as f32 / source_h as f32)
            .min(1.0);
        let target_w = ((source_w as f32 * scale).round() as u32).max(1).min(source_w);
        let target_h = ((source_h as f32 * scale).round() as u32).max(1).min(source_h);

        let mut data = vec![0u8; (target_w * target_h * 4) as usize];
        for y in 0..target_h {
            let source_y = (y as u64 * source_h as u64 / target_h as u64) as u32;
            for x in 0..target_w {
                let source_x = (x as u64 * source_w as u64 / target_w as u64) as u32;
                let from = ((source_y * source_w + source_x) * 4) as usize;
                let to = ((y * target_w + x) * 4) as usize;
                data[to..to + 4].copy_from_slice(&self.data[from..from + 4]);
            }
        }
        Frame::from_rgba(Size::new(target_w, target_h), data)
            .expect("缩略图尺寸与缓冲区长度一致")
    }

    /// 喂入下一帧。
    pub fn push(&mut self, next: Frame, options: &MatchOptions) -> PushOutcome {
        if next.size() != self.last.size() {
            // 尺寸变了说明外面没保持住区域，直接不认这一帧
            return PushOutcome::NoMatch;
        }
        if self.is_full() {
            return PushOutcome::TooTall;
        }

        let Some(result) = find_vertical_offset(&self.last, &next, options) else {
            // 正向没匹配上时，试一下反向——用户可能往回滚了。
            //
            // 反向匹配的含义：把 `next` 当作基准，`self.last` 的内容相对它下移了 offset 行，
            // 也就是**用户往上滚了**，新的内容出现在当前帧的**顶部**。
            match find_vertical_offset(&next, &self.last, options) {
                Some(reverse) if reverse.offset >= options.min_offset => {
                    return self.prepend(next, reverse.offset, reverse);
                }
                Some(_) => return PushOutcome::Unchanged,
                None => {
                    self.last_match = None;
                    return PushOutcome::NoMatch;
                }
            }
        };

        if result.offset < options.min_offset {
            // 理论上 find_vertical_offset 不会返回这么小的值，保险起见
            self.last = next;
            self.last_match = Some(result);
            return PushOutcome::Unchanged;
        }

        // 窗口往下移了 offset 行：两侧余量跟着变。
        // 真正要追加的是"offset 减去画布下方已经有的余量"——用户往下滚回已经拍过的内容时，
        // 这个差是 0，画布一点都不该长（否则长图里会出现重复段落）。
        let added = result.offset.saturating_sub(self.tail_room);
        let append = added.min(self.max_height.saturating_sub(self.height));
        // **空白行不能算新内容**：一片同样的底色在任何位移下都"匹配"，所以两张几乎
        // 只有空白的画面能凑出任意位移。真机上表现为「横向滚一张只有一行字的画面，
        // 长图高度凭空多出一行字的高度，而且方向从此被锁死在纵向」——
        // 0.1.49 收尾跑 `scroll_horizontal_e2e` 就是这么失败的。
        if append > 0 && !band_has_texture(&next, next.height() - append, next.height()) {
            // 返回 `NoMatch` 而不是 `Unchanged`：这不是"滚动了一下但没新内容"，
            // 而是"这个位移整个是假的"。上层 `DirectedStitcher` 正是靠 `NoMatch`
            // 才去试另一个方向——真机上的横向滚动就指着它（0.1.49 收尾的探针）。
            log::debug!("位移 {offset} 匹配上了，但要追加的是空白行，判定为误匹配", offset = result.offset);
            self.last_match = None;
            return PushOutcome::NoMatch;
        }
        self.head_room = self.head_room.saturating_add(result.offset);
        self.tail_room = self.tail_room.saturating_sub(result.offset);
        if append == 0 {
            self.last = next;
            self.last_match = Some(result);
            return PushOutcome::Unchanged;
        }

        // 注意：追加的是**当前帧**底部那 append 行（画布底之后的那部分才是新内容）
        let next_start = ((next.height() - append) * self.width * 4) as usize;
        let next_end = (next.height() * self.width * 4) as usize;
        self.data.extend_from_slice(&next.data()[next_start..next_end]);
        self.height += append;
        // 被长度上限截掉的部分仍然算"画布下方还没收进来的余量"
        self.tail_room = added - append;

        self.last = next;
        self.frames += 1;
        self.last_match = Some(result);
        PushOutcome::Appended {
            offset: result.offset,
        }
    }

    /// 把当前帧**顶部**新露出的 `offset` 行插到画布开头（用户往回滚时走这里）。
    ///
    /// 代价说明：这是**前插**，必须把已有数据整体后移一次（`Vec` 没有前插的摊销优化）。
    /// 一帧最多移动一次整张画布，实测量级是几十 MB 内存拷贝，对这个交互（用户手动滚）
    /// 完全可以接受；真正要避免的是"每帧都复制整张画布"（那才是平方级开销），
    /// 所以这里只做**一次** `extend + extend`。
    fn prepend(&mut self, next: Frame, offset: u32, result: MatchResult) -> PushOutcome {
        // 真正要前插的是"offset 减去画布上方已经有的余量"：用户往回滚到已经拍过的内容时，
        // 这个差是 0，画布一点都不该长（否则长图里会出现重复段落）。
        let added = offset.saturating_sub(self.head_room);
        let prepend = added.min(self.max_height.saturating_sub(self.height));
        // 同 `push`：要插到开头的那几行如果全是空白，这个"往回滚"就是误匹配。
        if prepend > 0 && !band_has_texture(&next, 0, prepend) {
            log::debug!("反向位移 {offset} 匹配上了，但要前插的是空白行，判定为误匹配");
            self.last_match = None;
            return PushOutcome::NoMatch;
        }
        self.tail_room = self.tail_room.saturating_add(offset);
        self.head_room = self.head_room.saturating_sub(offset);
        if prepend == 0 {
            self.last = next;
            self.last_match = Some(result);
            return if added == 0 {
                PushOutcome::Unchanged
            } else {
                PushOutcome::TooTall
            };
        }
        // 当前帧顶部那 prepend 行是"更早"的内容
        let head = (prepend * self.width * 4) as usize;
        let mut merged = Vec::with_capacity(self.data.len() + head);
        merged.extend_from_slice(&next.data()[..head]);
        merged.extend_from_slice(&self.data);
        self.data = merged;
        self.height += prepend;
        // 被长度上限截掉的部分仍然算"画布上方还没收进来的余量"
        self.head_room = added - prepend;
        self.last = next;
        self.frames += 1;
        self.last_match = Some(result);
        PushOutcome::Prepended { offset }
    }

    /// 转置整个拼接器（横纵方向互换时用）：数据与"上一帧"一起转置，宽高互换。
    ///
    /// 上限 `max_height` 原样沿用——它在哪个方向上都表示"长度上限"。
    #[must_use]
    pub fn transposed(&self) -> Self {
        let (w, h) = (self.width, self.height);
        let mut data = vec![0u8; self.data.len()];
        for y in 0..h {
            for x in 0..w {
                let from = ((y * w + x) * 4) as usize;
                let to = ((x * h + y) * 4) as usize;
                data[to..to + 4].copy_from_slice(&self.data[from..from + 4]);
            }
        }
        Self {
            width: h,
            height: w,
            data,
            max_height: self.max_height,
            last: self.last.transposed(),
            frames: self.frames,
            last_match: self.last_match,
            head_room: self.head_room,
            tail_room: self.tail_room,
        }
    }
}

/// 便捷入口：把一串帧拼成一张长图。
///
/// 主要用于测试与批处理；交互式采集由宿主按帧调用 [`ScrollStitcher::push`]。
pub fn stitch(frames: &[Frame], options: &MatchOptions, max_height: u32) -> Result<ScrollStitcher> {
    let Some(first) = frames.first() else {
        return Err(Error::Unsupported("滚动截屏至少需要一帧"));
    };
    let mut stitcher = ScrollStitcher::new(first.clone(), max_height);
    for frame in &frames[1..] {
        stitcher.push(frame.clone(), options);
    }
    Ok(stitcher)
}

// ---------------------------------------------------------------------------
// 滚动方向：横向复用纵向那套拼接
// ---------------------------------------------------------------------------

/// 内容滚动的方向。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    /// 上下滚（默认）。
    #[default]
    Vertical,
    /// 左右滚。
    Horizontal,
}

impl Axis {
    /// 另一个方向。
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Vertical => "纵向",
            Self::Horizontal => "横向",
        }
    }
}

/// 这块区域是不是"有内容"（不是整块同色）。
///
/// 空白区域（记事本里没字的行、纯色背景）在**任何**位移下都能匹配上，于是"换个方向试试"
/// 会误判——这正是把一次纵向滚动判成横向的根源（实测：840 宽的长图被拼成 1032 宽）。
fn band_has_texture(frame: &Frame, y0: u32, y1: u32) -> bool {
    let mut first: Option<[u8; 4]> = None;
    let mut different = 0u32;
    for y in (y0..y1.min(frame.height())).step_by(4) {
        for x in (0..frame.width()).step_by(8) {
            let px = frame.pixel(x as i32, y as i32);
            match first {
                None => first = Some(px),
                Some(base) => {
                    if px != base {
                        different += 1;
                        if different >= 8 {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// **带方向的拼接器：横向滚动直接复用纵向那套逻辑。**
///
/// 做法是把每一帧转置之后再喂给 [`ScrollStitcher`]（在转置坐标系里，"横向滚动"
/// 与"纵向滚动"是同一件事），取图/缩略图时再转置回来。好处是匹配、接缝、长度上限
/// 这些最容易写错的地方一行都不用改、也不用再测一遍。
///
/// **方向是自动判出来的**：当前方向匹配不上时试一下另一个方向，能接上就整体转置切换。
/// 用户不需要先告诉程序"我要横着滚"——他滚哪边，程序就往哪边拼。
#[derive(Debug)]
pub struct DirectedStitcher {
    axis: Axis,
    inner: ScrollStitcher,
    options: MatchOptions,
    /// "另一个方向连续匹配上了几次"。
    ///
    /// 一次偶然的误匹配不足以翻方向。实测就是这么翻车的：纵向探针里连续 14 次滚轮
    /// 让两帧完全没有重叠，而记事本内容里那些**等长的横线**在转置坐标系里任何位移都能
    /// "匹配"，于是 840 宽的长图被拼成了 1019 宽。连续两次才算数之后这种情况就过不去了
    /// （真横向滚动每帧都匹配，两帧之内必然切换）。
    flip_votes: u32,
    /// 第一次投票时那一帧（已经转过置）。
    ///
    /// 两个作用：第二次投票要拿它当参照（不能再拿"上一次成功拼接的帧"，否则位移会越攒越大、
    /// 直到超出帧高而永远匹配不上）；确认切换时要把它补进拼接，接缝才是连续的。
    flip_reference: Option<Frame>,
    /// 第一次投票时的位移：第二次必须**步长一致**才算确认。
    ///
    /// 真横向滚动每帧步长差不多；周期性内容（等长横线、表格分隔线）凑出来的
    /// 误匹配步长是随机的，靠这一条也拦得住。
    flip_offset: u32,
}

impl DirectedStitcher {
    /// 以第一帧为起点。`max_length` 是**长图长度上限**（纵向是高度、横向是宽度）。
    #[must_use]
    pub fn new(first: Frame, max_length: u32, options: MatchOptions) -> Self {
        Self {
            axis: Axis::Vertical,
            inner: ScrollStitcher::new(first, max_length),
            options,
            flip_votes: 0,
            flip_reference: None,
            flip_offset: 0,
        }
    }

    /// 当前判定的滚动方向。
    #[must_use]
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// 长图尺寸（已按方向还原成"用户看到的那个方向"）。
    #[must_use]
    pub fn size(&self) -> Size {
        let size = self.inner.size();
        match self.axis {
            Axis::Vertical => size,
            Axis::Horizontal => Size::new(size.h, size.w),
        }
    }

    #[must_use]
    pub fn frames(&self) -> u32 {
        self.inner.frames()
    }

    #[must_use]
    pub fn last_match(&self) -> Option<MatchResult> {
        self.inner.last_match()
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    /// 长图（横向时转置回正常朝向）。
    #[must_use]
    pub fn image(&self) -> Frame {
        let image = self.inner.image();
        match self.axis {
            Axis::Vertical => image,
            Axis::Horizontal => image.transposed(),
        }
    }

    /// 缩略图（横向时转置回正常朝向；尺寸上限按**用户看到的朝向**解释）。
    #[must_use]
    pub fn thumbnail(&self, max: Size) -> Frame {
        let (max_w, max_h) = match self.axis {
            Axis::Vertical => (max.w, max.h),
            // 转置坐标系里宽高互换
            Axis::Horizontal => (max.h, max.w),
        };
        let thumb = self.inner.thumbnail(Size::new(max_w, max_h));
        match self.axis {
            Axis::Vertical => thumb,
            Axis::Horizontal => thumb.transposed(),
        }
    }

    /// 推进一帧：先按当前方向试，接不上就试另一个方向（连续两次都接得上才切换）。
    /// 交给内层拼接器的帧：横向时必须是**转置后**的。
    ///
    /// ⚠️ 漏掉这一步会让切换方向之后的每一帧都因为"尺寸对不上"被丢掉——
    /// 症状是横向长图只涨过切换那一次就不动了。单测里断言精确宽度时才发现
    /// （`reverse_horizontal_scroll_is_detected_and_prepended`、
    /// `scrolling_down_then_back_up_covers_the_whole_range`）。
    fn for_inner(&self, next: Frame) -> Frame {
        match self.axis {
            Axis::Vertical => next,
            Axis::Horizontal => next.transposed(),
        }
    }

    pub fn push(&mut self, next: Frame) -> PushOutcome {
        let outcome = self.inner.push(self.for_inner(next.clone()), &self.options);
        if outcome != PushOutcome::NoMatch {
            // 当前方向是好的，把"想翻方向"的票清掉。
            //
            // **`Unchanged` 不算"好"**：它只表示这一帧和上一帧一样（用户两次滚轮之间
            // 抓到的静止帧）。抓帧间隔比滚轮间隔短，这种帧很常见；如果它也清票，
            // 两次投票就几乎永远排不到一起——真机上表现为「横向滚动有时拼得出来、
            // 有时一行都不拼」（0.1.49 收尾跑 `scroll_horizontal_e2e` 时好时坏）。
            if !matches!(outcome, PushOutcome::Unchanged) {
                self.flip_votes = 0;
            }
            return outcome;
        }
        // 只在"还没成功拼进任何内容"之前允许换方向。
        //
        // 一旦当前方向已经拼进去过，说明方向判断是对的；此时某两帧接不上往往只是
        // "这一帧滚得太远"（滚轮一次跳好几屏就会这样）。此刻去试另一个方向很危险：
        // 空白区域在任何位移下都"匹配"，于是会把纵向滚动判成横向。
        if self.inner.frames() > 1 {
            return PushOutcome::NoMatch;
        }

        // 横向时把两边都转置，纵向时原样——两种情况的判断逻辑因此是同一条
        let (last, next_flipped) = match self.axis {
            Axis::Vertical => (self.inner.last.clone().transposed(), next.transposed()),
            Axis::Horizontal => (self.inner.last.clone(), next.clone()),
        };
        // 第二次投票要拿"第一次投票那一帧"当参照，不能再拿上一次成功拼接的帧：
        // 后者会让位移一帧一帧攒大，很快超过帧高，于是永远匹配不上（第一版就是这样，
        // 单测里横向四帧的内容只在第一帧之后就再也接不上了）。
        let reference = self.flip_reference.clone().unwrap_or(last);
        // 正向、反向都算"这个方向接得上"：反向（用户往回滚）同样说明该换成这个方向。
        let offset = match (
            find_vertical_offset(&reference, &next_flipped, &self.options),
            find_vertical_offset(&next_flipped, &reference, &self.options),
        ) {
            (Some(forward), _) if forward.offset >= self.options.min_offset => forward.offset,
            (_, Some(reverse)) if reverse.offset >= self.options.min_offset => reverse.offset,
            // 位移小于阈值等于"没滚动"（两帧一模一样时也会给出 0），绝不能算一次投票——
            // 否则静止画面连来两帧就把方向翻了。
            _ => {
                self.flip_votes = 0;
                self.flip_reference = None;
                return PushOutcome::NoMatch;
            }
        };
        // **还要求重叠区里有真内容**：纯色区域任何位移都能"匹配"，那种匹配毫无意义
        let overlap_start = next_flipped.height().saturating_sub(offset);
        if !band_has_texture(&next_flipped, overlap_start, next_flipped.height()) {
            log::debug!("另一个方向虽然能匹配上，但重叠区是纯色，判定为误匹配，保持当前方向");
            self.flip_votes = 0;
            self.flip_reference = None;
            return PushOutcome::NoMatch;
        }

        // 一次不算数：等下一帧也接得上、而且**步长一致**再翻（见 `flip_votes` 的说明）
        if self.flip_reference.is_none() {
            self.flip_votes = 1;
            self.flip_offset = offset;
            self.flip_reference = Some(next_flipped);
            log::debug!("另一个方向接上了（第 1 次，位移 {offset}），再确认一次再翻方向");
            return PushOutcome::NoMatch;
        }
        let tolerance = (self.flip_offset / 4).max(4);
        // 步长一致才确认，但"一致"要宽一点：抓帧比滚轮慢时，第二票看到的可能是
        // **两次滚轮**的位移（120 → 240）。倍数关系同样说明是同一个滚轮在推同一段内容，
        // 而随机误匹配凑出整数倍的概率很低。
        let (big, small) = (
            offset.max(self.flip_offset),
            offset.min(self.flip_offset),
        );
        let same_step =
            big.abs_diff(small) <= tolerance || (small > 0 && big % small <= tolerance);
        if !same_step {
            log::debug!(
                "另一个方向虽然又接上了，但步长不一致（{offset} vs {}），判定为误匹配",
                self.flip_offset
            );
            self.flip_votes = 0;
            self.flip_reference = None;
            return PushOutcome::NoMatch;
        }
        self.flip_votes = 0;

        // 能接上：把已经攒下的画布整体转置，然后按新方向继续。
        //
        // ⚠️ 切换之后喂进去的帧**也必须转置**：`ScrollStitcher` 在尺寸不一致时会直接
        // 拒绝这一帧（第一版就是这里漏了转置，于是方向切对了、宽度却一直不涨）。
        //
        // ⚠️ 还要**补上第一次投票那一帧**：它才是紧挨着画布末尾的那一帧。少了它，
        // 画布与当前帧之间就多出一个空档，位移超出帧高，切了方向也拼不进去。
        let pending = self.flip_reference.take();
        let flipped = self.inner.transposed();
        self.axis = self.axis.flipped();
        self.inner = flipped;
        log::info!("滚动截屏方向切换为：{}", self.axis.label());
        if let Some(pending) = pending {
            let _ = self.inner.push(pending, &self.options);
        }
        self.inner.push(self.for_inner(next), &self.options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;

    /// 造一张「每行都独一无二」的图：用线性同余生成器铺像素。
    /// 这样任意位移的匹配都是唯一的，测试才有意义。
    fn noisy(width: u32, height: u32, seed: u32) -> Frame {
        let mut state = seed | 1;
        let mut data = vec![0u8; (width * height * 4) as usize];
        for pixel in data.as_chunks_mut::<4>().0 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = (state >> 24) as u8;
            pixel[0] = value;
            pixel[1] = value.wrapping_add(37);
            pixel[2] = value.wrapping_add(91);
            pixel[3] = 255;
        }
        Frame::from_rgba(Size::new(width, height), data).unwrap()
    }

    /// 从一张大图里取一个窗口。
    fn window(source: &Frame, top: u32, height: u32) -> Frame {
        source.crop(Rect::new(0, top as i32, source.width(), height))
    }

    #[test]
    fn sticky_prefix_detects_identical_leading_rows() {
        let mut a = Frame::filled(Size::new(32, 20), [10, 10, 10, 255]);
        let mut b = a.clone();
        // 前 5 行相同，第 6 行起不同
        for y in 5..20 {
            a.fill_rect(Rect::new(0, y, 32, 1), [y as u8, 0, 0, 255]);
            b.fill_rect(Rect::new(0, y, 32, 1), [(y + 1) as u8, 0, 0, 255]);
        }
        assert_eq!(sticky_prefix_rows(&a, &b, 0), 5);
    }

    #[test]
    fn sticky_prefix_is_zero_for_unrelated_frames() {
        let a = noisy(32, 20, 1);
        let b = noisy(32, 20, 2);
        assert_eq!(sticky_prefix_rows(&a, &b, 0), 0);
    }

    #[test]
    fn sticky_prefix_requires_matching_sizes() {
        let a = noisy(32, 20, 1);
        let b = noisy(32, 24, 1);
        assert_eq!(sticky_prefix_rows(&a, &b, 0), 0);
    }

    #[test]
    fn offset_is_recovered_from_a_known_shift() {
        let tall = noisy(200, 600, 7);
        let prev = window(&tall, 0, 300);
        let next = window(&tall, 13, 300);
        let result = find_vertical_offset(&prev, &next, &MatchOptions::default())
            .expect("应当能求出位移");
        assert_eq!(result.offset, 13, "位移必须精确等于真实滚动量");
        assert!(result.mean_error < 0.5, "同一张图的内容应当几乎无误差");
    }

    #[test]
    fn offset_search_covers_a_range_of_shifts() {
        let tall = noisy(160, 900, 11);
        // 注意从 min_offset（默认 2）开始：1 像素的位移被刻意当成"没滚动"，
        // 见下面的 `sub_threshold_shift_counts_as_no_scroll`
        for shift in [2u32, 3, 5, 8, 21, 64, 137, 299] {
            let prev = window(&tall, 0, 400);
            let next = window(&tall, shift, 400);
            let result = find_vertical_offset(&prev, &next, &MatchOptions::default())
                .unwrap_or_else(|| panic!("位移 {shift} 应当能求出来"));
            assert_eq!(result.offset, shift, "位移 {shift} 求成了 {}", result.offset);
        }
    }

    #[test]
    fn sub_threshold_shift_counts_as_no_scroll() {
        // 1 像素的位移低于 min_offset：滚轮/触摸板抖动、亚像素渲染都会造成这种量级的
        // 差异，把它当成滚动会导致长图里塞进大量 1 像素的重复行。
        let tall = noisy(160, 400, 12);
        let prev = window(&tall, 0, 300);
        let next = window(&tall, 1, 300);
        let options = MatchOptions::default();
        assert_eq!(options.min_offset, 2);
        assert!(find_vertical_offset(&prev, &next, &options).is_none());
    }

    #[test]
    fn identical_frames_yield_no_offset() {
        let frame = noisy(120, 200, 3);
        // 完全没滚动：正向找不到 >= min_offset 的位移
        assert!(find_vertical_offset(&frame, &frame, &MatchOptions::default()).is_none());
    }

    #[test]
    fn unrelated_frames_are_rejected() {
        let a = noisy(120, 300, 4);
        let b = noisy(120, 300, 99);
        assert!(
            find_vertical_offset(&a, &b, &MatchOptions::default()).is_none(),
            "内容完全不同的两帧不应给出位移"
        );
    }

    #[test]
    fn sticky_header_does_not_break_the_match() {
        // 顶部 40 行固定不动（导航栏），下面才是滚动内容
        let tall = noisy(200, 700, 21);
        let mut prev = window(&tall, 0, 300);
        let mut next = window(&tall, 17, 300);
        let header = noisy(200, 40, 555);
        prev.blit(&header, crate::geom::Point::ZERO);
        next.blit(&header, crate::geom::Point::ZERO);

        let result = find_vertical_offset(&prev, &next, &MatchOptions::default()).unwrap();
        assert_eq!(result.offset, 17, "排除粘性页头后仍应求出真实位移");
        assert!(result.sticky_rows >= 38, "应当识别出约 40 行粘性页头");
    }

    #[test]
    fn fully_periodic_content_is_reported_as_undetectable() {
        // 整帧都是周期图案（这里周期 100）时，「滚了 100 像素」与「没滚」在像素上
        // **完全等价**——这是本质上无法判定的情形。
        //
        // 实现的选择是报告"没匹配上"，而不是猜一个位移：猜错的代价是一张错位或重复的
        // 长图，而报告"没识别到滚动"至少让用户知道发生了什么。
        // 换句话说，这里失败是**正确行为**，不是 bug。
        let period = 100u32;
        let height = 400u32;
        let width = 120u32;
        let mut data = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            let value = (y % period) as u8;
            for x in 0..width {
                let offset = ((y * width + x) * 4) as usize;
                data[offset] = value;
                data[offset + 1] = value.wrapping_add(60);
                data[offset + 2] = value.wrapping_add(120);
                data[offset + 3] = 255;
            }
        }
        let tall = Frame::from_rgba(Size::new(width, height), data).unwrap();
        let prev = window(&tall, 0, 300);
        let next = window(&tall, period, 300);

        // 先确认这确实是"整帧看起来没变"
        assert_eq!(
            sticky_prefix_rows(&prev, &next, 0),
            prev.height(),
            "周期性内容会让整帧与上一帧逐行相同"
        );
        // 因此实现应当明确报告"无法判定"，而不是猜一个位移
        assert!(
            find_vertical_offset(&prev, &next, &MatchOptions::default()).is_none(),
            "整帧周期性等价时不应给出位移猜测"
        );
    }

    #[test]
    fn stitcher_appends_exactly_the_revealed_rows() {
        let tall = noisy(160, 800, 31);
        let viewport = 300;
        let step = 60;

        let mut stitcher = ScrollStitcher::new(window(&tall, 0, viewport), 20_000);
        let frames = 5;
        for index in 1..=frames {
            let next = window(&tall, index * step, viewport);
            match stitcher.push(next, &MatchOptions::default()) {
                PushOutcome::Appended { offset } => assert_eq!(offset, step),
                other => panic!("第 {index} 帧应当拼接成功，实际 {other:?}"),
            }
        }

        assert_eq!(stitcher.size().w, 160);
        assert_eq!(
            stitcher.size().h,
            viewport + step * frames,
            "长图高度 = 首帧 + 每次新露出的行数"
        );

        // 拼出来的长图必须与原始大图的对应区域**逐像素一致**
        let expected = tall.crop(Rect::new(0, 0, 160, viewport + step * frames));
        assert_eq!(
            stitcher.image().data(),
            expected.data(),
            "拼接结果应当与真实内容逐像素相同"
        );
    }

    #[test]
    fn dropped_frames_do_not_leave_gaps_or_duplicates() {
        // 采集线程每 250ms 才抓一帧，用户滚得快时中间那些位置根本没被拍到。
        // 这是**正常情况**，长图必须依然和真实内容逐像素一致：
        // 只要匹配始终以「上一张被接受的帧」为基准，跳过的帧就等价于一次更大的位移。
        let tall = noisy(160, 900, 61);
        let viewport = 200;
        let mut stitcher = ScrollStitcher::new(window(&tall, 0, viewport), 20_000);

        // 只喂奇数倍的位移：0 -> 120 -> 240 -> 360（中间 60/180/300 三帧被"漏掉"）
        for (index, top) in [(1u32, 120u32), (2, 240), (3, 360)] {
            let next = window(&tall, top, viewport);
            match stitcher.push(next, &MatchOptions::default()) {
                PushOutcome::Appended { offset } => assert_eq!(
                    offset, 120,
                    "第 {index} 次推送应当识别出 120 像素的位移"
                ),
                other => panic!("第 {index} 次推送应当拼接成功，实际 {other:?}"),
            }
        }

        let expected_height = viewport + 360;
        assert_eq!(stitcher.size().h, expected_height, "丢帧不应改变总高度");
        let expected = tall.crop(Rect::new(0, 0, 160, expected_height));
        assert_eq!(
            stitcher.image().data(),
            expected.data(),
            "丢帧后拼接结果仍必须与真实内容逐像素一致（既不缺行也不重复）"
        );
    }

    #[test]
    fn a_frame_that_jumps_beyond_the_search_range_is_skipped_without_corrupting() {
        // 位移超过 `max_offset` 时匹配不上：只能跳过。跳过后长图仍是**正确的前缀**，
        // 不能出现错位的行——后续帧一旦重新匹配上，拼接要继续正确。
        let tall = noisy(160, 900, 71);
        let viewport = 200;
        let options = MatchOptions {
            max_offset: 100,
            ..MatchOptions::default()
        };
        let mut stitcher = ScrollStitcher::new(window(&tall, 0, viewport), 20_000);

        // 一步跳 300 像素，超出搜索范围
        let jumped = window(&tall, 300, viewport);
        assert_eq!(
            stitcher.push(jumped, &options),
            PushOutcome::NoMatch,
            "位移超出 max_offset 时应当报告未匹配"
        );
        assert_eq!(stitcher.size().h, viewport, "未匹配时不应改变长图");

        // 接着按可达的步长继续滚：位移基准仍是最初那帧，两帧相距 300 + 80
        let next = window(&tall, 380, viewport);
        assert_eq!(
            stitcher.push(next, &options),
            PushOutcome::NoMatch,
            "累计位移仍超出范围，继续报告未匹配"
        );

        // 等它回到范围内：从最初那帧算起 90 像素
        let back = window(&tall, 90, viewport);
        match stitcher.push(back, &options) {
            PushOutcome::Appended { offset } => assert_eq!(offset, 90),
            other => panic!("回到搜索范围内后应当重新匹配上，实际 {other:?}"),
        }
        let expected = tall.crop(Rect::new(0, 0, 160, viewport + 90));
        assert_eq!(
            stitcher.image().data(),
            expected.data(),
            "跳过若干帧之后，已经拼好的部分必须仍然是真实内容的正确前缀"
        );
    }

    #[test]
    fn stitcher_reports_unchanged_when_nothing_scrolled() {
        let tall = noisy(120, 400, 41);
        let frame = window(&tall, 0, 200);
        let mut stitcher = ScrollStitcher::new(frame.clone(), 20_000);
        assert_eq!(
            stitcher.push(frame, &MatchOptions::default()),
            PushOutcome::NoMatch,
            "完全没滚动时正向匹配不到位移，应当报告未拼接（而不是重复追加）"
        );
        assert_eq!(stitcher.size().h, 200, "长图不应变长");
    }

    #[test]
    fn a_match_that_would_append_blank_rows_is_not_a_scroll() {
        // 横向滚动的内容在纵向匹配器眼里是这样的：只有顶上那一条字在左右移动，
        // 下面全是同样的底色。底色在任何位移下都能「匹配」，于是能凑出一个位移，
        // 但那个位移要追加的行全是空白 —— 它不是滚动。
        //
        // 真机症状（0.1.49 收尾跑 `scroll_horizontal_e2e`）：一行字横着移出采样范围后，
        // 剩下的全是一片底色，纵向匹配器照样"匹配"上一个位移，长图高度从 320 变成 344
        // （正好多出一行字的高度），而且方向从此被锁死在纵向、横向再也拼不进去。
        //
        // 单测里直接构造这一幕：上一帧顶上有一条黑带，下一帧整片底色。
        let mut first = Frame::filled(Size::new(40, 60), [255, 255, 255, 255]);
        first.fill_rect(crate::geom::Rect::new(5, 0, 10, 10), [0, 0, 0, 255]);
        let second = Frame::filled(Size::new(40, 60), [255, 255, 255, 255]);
        let options = MatchOptions::default();
        let mut stitcher = ScrollStitcher::new(first.clone(), 20_000);
        assert_eq!(stitcher.push(first, &options), PushOutcome::NoMatch);
        assert_eq!(
            stitcher.push(second, &options),
            PushOutcome::NoMatch,
            "要追加的行是空白，说明这个位移整个是假的，应当报「没识别出滚动」"
        );
        assert_eq!(stitcher.size().h, 60, "长图不该因为一片空白就变高");
    }

    #[test]
    fn stitcher_prepends_when_scrolling_backwards() {
        let tall = noisy(120, 500, 51);
        let mut stitcher = ScrollStitcher::new(window(&tall, 100, 200), 20_000);
        // 往回滚：视口上移到第 60 行，新露出来的是**顶部**那 40 行
        let backwards = window(&tall, 60, 200);
        assert_eq!(
            stitcher.push(backwards, &MatchOptions::default()),
            PushOutcome::Prepended { offset: 40 }
        );
        // 画布变成 240 行，且与真值逐像素一致（内容顺序必须是"更早的在上"）
        assert_eq!(stitcher.size().h, 240);
        let truth = tall.crop(crate::geom::Rect::new(0, 60, 120, 240));
        assert_eq!(stitcher.image().data(), truth.data(), "反向拼接后内容顺序不对");
    }

    #[test]
    fn stitcher_stops_at_the_height_limit() {
        let tall = noisy(100, 2_000, 61);
        let mut stitcher = ScrollStitcher::new(window(&tall, 0, 200), 250);
        let mut outcome = PushOutcome::Appended { offset: 0 };
        for index in 1..=10 {
            outcome = stitcher.push(window(&tall, index * 20, 200), &MatchOptions::default());
            if outcome == PushOutcome::TooTall {
                break;
            }
        }
        assert_eq!(outcome, PushOutcome::TooTall, "达到上限后应当停止追加");
        assert!(stitcher.size().h <= 250, "长图不得超过上限");
        assert!(stitcher.is_full());
    }

    #[test]
    fn size_mismatch_is_rejected() {
        let mut stitcher = ScrollStitcher::new(noisy(100, 200, 71), 20_000);
        let wrong = noisy(120, 200, 72);
        assert_eq!(
            stitcher.push(wrong, &MatchOptions::default()),
            PushOutcome::NoMatch
        );
    }

    #[test]
    fn thumbnail_shrinks_to_the_requested_box_and_keeps_the_aspect() {
        let tall = noisy(400, 1000, 81);
        let stitcher = ScrollStitcher::new(tall, 20_000);

        let thumb = stitcher.thumbnail(Size::new(100, 100));
        assert!(thumb.width() <= 100 && thumb.height() <= 100);
        // 400x1000 缩到 100 宽 → 250 高（高受 max_h 限制时反过来）
        assert_eq!(thumb.width(), 40, "1000 高超过上限，应当由高度决定缩放比");
        assert_eq!(thumb.height(), 100);
        assert_eq!(thumb.data().len(), (thumb.width() * thumb.height() * 4) as usize);
    }

    #[test]
    fn thumbnail_never_upscales() {
        let small = noisy(20, 30, 82);
        let stitcher = ScrollStitcher::new(small, 20_000);
        let thumb = stitcher.thumbnail(Size::new(4000, 4000));
        assert_eq!(thumb.size(), Size::new(20, 30), "小图不该被放大");
    }

    #[test]
    fn thumbnail_of_the_first_frame_matches_it() {
        let frame = noisy(64, 48, 83);
        let stitcher = ScrollStitcher::new(frame.clone(), 20_000);
        // 请求尺寸与原图一致时应当逐像素相同
        let thumb = stitcher.thumbnail(frame.size());
        assert_eq!(thumb.data(), frame.data());
    }

    /// 端到端：拿**真实的屏幕截图**当素材，模拟视口滚动，验证拼回来的长图与真值一致。
    ///
    /// 这一条是整块逻辑最有价值的测试：它同时覆盖位移估计与拼接顺序，
    /// 而且用的是真实像素而不是合成图案。
    #[test]
    #[ignore = "需要交互式桌面（真的截一张屏当素材）"]
    fn real_screenshot_scroll_roundtrip() {
        use crate::capture::desktop_capture;

        let backend = desktop_capture().expect("捕获后端不可用");
        let shot = backend.capture_desktop().expect("截屏失败");
        let source = shot.frame;
        assert!(source.height() > 600, "屏幕太矮，无法做滚动模拟");

        let viewport_height = 400u32;
        let step = 37u32;
        let width = source.width().min(1200);
        let source = source.crop(Rect::new(0, 0, width, source.height()));

        let mut stitcher =
            ScrollStitcher::new(window(&source, 0, viewport_height), 20_000);
        let frames = 6;
        for index in 1..=frames {
            let top = index * step;
            if top + viewport_height > source.height() {
                break;
            }
            let outcome = stitcher.push(window(&source, top, viewport_height), &MatchOptions::default());
            println!("第 {index} 帧：{outcome:?}");
        }

        let expected_height = viewport_height + step * frames;
        let expected = source.crop(Rect::new(0, 0, width, expected_height));
        println!(
            "长图 {}x{}，真值 {}x{}",
            stitcher.size().w,
            stitcher.size().h,
            expected.width(),
            expected.height()
        );

        assert_eq!(stitcher.size().h, expected_height);
        // 真实截图上允许极小的差异（行签名抽稀带来的边界舍入），但不允许错位
        let got = stitcher.image();
        let mut mismatch = 0u64;
        for (a, b) in got
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.data().as_chunks::<4>().0)
        {
            if a[0].abs_diff(b[0]) > 8 {
                mismatch += 1;
            }
        }
        let ratio = mismatch as f64 / (got.width() as f64 * got.height() as f64);
        println!("不一致像素占比 {:.4}%", ratio * 100.0);
        assert!(ratio < 0.001, "拼接结果与真值差异过大：{:.4}%", ratio * 100.0);
    }
}

// ---------------------------------------------------------------------------
// 方向自动判定（横向滚动截屏）
// ---------------------------------------------------------------------------


/// 自动滚动什么时候该停。
///
/// 自动模式会替用户合成滚轮事件，所以"什么时候停"必须是个能单独测的东西：
/// 连着几帧画面没变化（到底了 / 目标程序把滚轮吃了）、或者已经转发到长度上限，
/// 就该停下来，而不是一直往人家窗口里灌事件。
#[derive(Debug, Clone, Copy)]
pub struct AutoScrollTracker {
    /// 连续多少帧"没有新增内容"就停。
    pub stall_frames: u32,
    /// 连续没新增的计数。
    stalled: u32,
    /// 已经转发过多少次滚轮（自保上限，防止无限灌事件）。
    pub steps: u32,
    /// 最多转发多少次。
    pub max_steps: u32,
}

/// 自动滚动的下一步该怎么走。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoScrollStep {
    /// 继续：发一次滚轮。
    Scroll,
    /// 停：画面连续没变化了（到底了，或目标程序忽略了合成滚轮）。
    Stalled,
    /// 停：已经转发到自保上限。
    Exhausted,
    /// 停：已经到长度上限。
    TooTall,
}

impl AutoScrollTracker {
    #[must_use]
    pub fn new(stall_frames: u32) -> Self {
        Self {
            stall_frames: stall_frames.max(1),
            stalled: 0,
            steps: 0,
            max_steps: 400,
        }
    }

    /// 喂一帧的结果，问下一步怎么办。
    pub fn observe(&mut self, outcome: PushOutcome) -> AutoScrollStep {
        match outcome {
            // 真的拼进去了：重置"没变化"的计数
            PushOutcome::Appended { .. } | PushOutcome::Prepended { .. } => {
                self.stalled = 0;
                self.steps += 1;
                if self.steps >= self.max_steps {
                    return AutoScrollStep::Exhausted;
                }
                AutoScrollStep::Scroll
            }
            // 画面没动 / 没认出滚动：可能是到底了，也可能目标程序忽略了合成滚轮。
            // 一次两次不算数（动画、懒加载都会这样），连着几次才停。
            PushOutcome::Unchanged | PushOutcome::NoMatch => {
                self.stalled += 1;
                self.steps += 1;
                if self.stalled >= self.stall_frames {
                    AutoScrollStep::Stalled
                } else if self.steps >= self.max_steps {
                    AutoScrollStep::Exhausted
                } else {
                    AutoScrollStep::Scroll
                }
            }
            PushOutcome::TooTall => AutoScrollStep::TooTall,
        }
    }
}

#[cfg(test)]
mod auto_scroll_tests {
    use super::*;

    #[test]
    fn keeps_scrolling_while_frames_keep_appending() {
        let mut tracker = AutoScrollTracker::new(3);
        for _ in 0..5 {
            assert_eq!(
                tracker.observe(PushOutcome::Appended { offset: 40 }),
                AutoScrollStep::Scroll
            );
        }
        assert_eq!(tracker.steps, 5);
    }

    #[test]
    fn stops_after_three_stalled_frames_in_a_row() {
        let mut tracker = AutoScrollTracker::new(3);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Scroll);
        assert_eq!(tracker.observe(PushOutcome::NoMatch), AutoScrollStep::Scroll);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Stalled);
    }

    #[test]
    fn a_good_frame_resets_the_stall_counter() {
        let mut tracker = AutoScrollTracker::new(3);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Scroll);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Scroll);
        // 又拼上了一帧：说明只是慢，不是到底了
        assert_eq!(
            tracker.observe(PushOutcome::Appended { offset: 10 }),
            AutoScrollStep::Scroll
        );
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Scroll);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Scroll);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Stalled);
    }

    #[test]
    fn stops_immediately_at_the_length_limit_and_at_the_step_budget() {
        let mut tracker = AutoScrollTracker::new(3);
        assert_eq!(tracker.observe(PushOutcome::TooTall), AutoScrollStep::TooTall);

        // 自保上限：一直有内容也不无限灌事件
        let mut tracker = AutoScrollTracker::new(3);
        tracker.max_steps = 4;
        for _ in 0..3 {
            assert_eq!(
                tracker.observe(PushOutcome::Appended { offset: 1 }),
                AutoScrollStep::Scroll
            );
        }
        assert_eq!(
            tracker.observe(PushOutcome::Appended { offset: 1 }),
            AutoScrollStep::Exhausted
        );
    }

    #[test]
    fn stall_frames_zero_is_treated_as_one() {
        let mut tracker = AutoScrollTracker::new(0);
        assert_eq!(tracker.observe(PushOutcome::Unchanged), AutoScrollStep::Stalled);
    }
}

#[cfg(test)]
mod directed_tests {
    use super::*;
    use crate::geom::Rect;

    /// 造一张"内容比视口大"的图：每个像素由一个 (x, y) 的散列决定。
    ///
    /// **必须用散列，不能用 `x*a + y*b` 这种线性图案**：线性图案有对角对称性——
    /// "横着移 60 列"在数值上等价于"竖着移某个行数"，于是横向滚动会被误判成纵向
    /// （第一版就是这么挂的：内容 `(x*7 + y*13) % 251`，横移 60 恰好等于竖移的某个倍数）。
    /// 散列让"只有真正的同一块内容"才会相等。
    fn content(width: u32, height: u32) -> Frame {
        let mut frame = Frame::new(Size::new(width, height));
        for y in 0..height {
            for x in 0..width {
                let mut h = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
                h ^= h >> 15;
                h = h.wrapping_mul(0x2545_F491);
                h ^= h >> 13;
                frame.set_pixel(
                    x as i32,
                    y as i32,
                    [(h & 0xFF) as u8, (h >> 8) as u8 | 0x80, (h >> 16) as u8, 255],
                );
            }
        }
        frame
    }

    /// 从大图里裁一块视口（模拟"屏幕只看得到一部分"）。
    fn window_of(big: &Frame, x: u32, y: u32, w: u32, h: u32) -> Frame {
        big.crop(Rect::new(x as i32, y as i32, w, h))
    }

    #[test]
    fn transposed_is_its_own_inverse() {
        let frame = content(37, 19);
        let round_trip = frame.transposed().transposed();
        assert_eq!(round_trip.size(), frame.size());
        assert_eq!(round_trip.data(), frame.data());
        assert_eq!(frame.transposed().size(), Size::new(19, 37));
    }

    #[test]
    fn vertical_scroll_stays_vertical_and_grows_in_height() {
        let big = content(120, 400);
        let viewport: Vec<Frame> = (0..5).map(|i| window_of(&big, 0, i * 60, 120, 100)).collect();
        let mut stitcher = DirectedStitcher::new(
            viewport[0].clone(),
            4000,
            MatchOptions::default(),
        );
        for frame in &viewport[1..] {
            stitcher.push(frame.clone());
        }
        assert_eq!(stitcher.axis(), Axis::Vertical, "纵向滚动不该被判成横向");
        let size = stitcher.size();
        assert_eq!(size.w, 120);
        assert!(size.h > 100, "应该拼出比视口更高的长图，实际 {size:?}");
    }

    #[test]
    fn horizontal_scroll_switches_axis_and_grows_in_width() {
        let big = content(400, 120);
        let viewport: Vec<Frame> = (0..5).map(|i| window_of(&big, i * 60, 0, 100, 120)).collect();
        let mut stitcher = DirectedStitcher::new(
            viewport[0].clone(),
            4000,
            MatchOptions::default(),
        );
        for frame in &viewport[1..] {
            stitcher.push(frame.clone());
        }
        assert_eq!(stitcher.axis(), Axis::Horizontal, "横向滚动应当自动切换方向");
        let size = stitcher.size();
        assert_eq!(size.h, 120, "横向长图的高度应保持视口高度");
        assert!(size.w > 100, "应该拼出比视口更宽的长图，实际 {size:?}");
        // 取图也要是"用户看到的方向"，不是转置过的
        assert_eq!(stitcher.image().size(), size);
        let thumb = stitcher.thumbnail(Size::new(64, 64));
        assert!(thumb.width() >= thumb.height(), "横向长图的缩略图该是宽的");
    }

    #[test]
    fn blank_content_does_not_switch_the_axis() {
        // 一片纯色：横向/纵向都"匹配得上"，但方向绝不能被改掉
        let blank = Frame::filled(Size::new(200, 200), [200, 200, 200, 255]);
        let mut stitcher = DirectedStitcher::new(blank.clone(), 4000, MatchOptions::default());
        for _ in 0..3 {
            stitcher.push(Frame::filled(Size::new(200, 200), [200, 200, 200, 255]));
        }
        assert_eq!(stitcher.axis(), Axis::Vertical, "纯色内容不该把方向判成横向");
    }

    #[test]
    fn axis_is_not_switched_after_the_first_append() {
        // 一旦某个方向已经拼进去过，就不该再改方向——某两帧接不上只是"滚太远了"
        let big = content(120, 600);
        let mut stitcher = DirectedStitcher::new(
            window_of(&big, 0, 0, 120, 100),
            4000,
            MatchOptions::default(),
        );
        stitcher.push(window_of(&big, 0, 60, 120, 100)); // 纵向接上，锁定纵向
        assert_eq!(stitcher.axis(), Axis::Vertical);
        // 给一帧"横向才接得上"的（模拟滚太远导致的误匹配机会），方向必须保持不变
        let mut wide = Frame::new(Size::new(120, 100));
        for y in 0..100i32 {
            for x in 0..120i32 {
                wide.set_pixel(x, y, [(x % 7) as u8 * 30, 10, 20, 255]);
            }
        }
        stitcher.push(wide);
        assert_eq!(stitcher.axis(), Axis::Vertical, "已经拼过内容之后不该改方向");
    }

    #[test]
    fn one_lucky_flipped_match_is_not_enough() {
        // 一帧"只有横向才接得上"的内容不足以翻方向：等下一帧确认
        let big = content(400, 120);
        let first = window_of(&big, 120, 0, 100, 120);
        let mut stitcher = DirectedStitcher::new(first, 4000, MatchOptions::default());
        // 横向平移 60：纵向匹配必然失败，转置之后能接上——只投一票
        stitcher.push(window_of(&big, 60, 0, 100, 120));
        assert_eq!(stitcher.axis(), Axis::Vertical, "一次匹配不该翻方向");
    }

    /// 反向（往上）滚动的三个场景：只用反向、只往一个方向、来回滚。
    #[test]
    fn reverse_vertical_scroll_prepends_content() {
        let big = content(120, 500);
        let ys = [200u32, 150, 100, 50];
        let frames: Vec<Frame> = ys.iter().map(|y| window_of(&big, 0, *y, 120, 100)).collect();
        let mut stitcher = DirectedStitcher::new(frames[0].clone(), 4000, MatchOptions::default());
        for frame in &frames[1..] {
            stitcher.push(frame.clone());
        }
        assert_eq!(stitcher.axis(), Axis::Vertical, "反向滚动不该改变方向判定");
        assert_eq!(stitcher.size(), Size::new(120, 250));
        let truth = window_of(&big, 0, 50, 120, 250);
        assert_eq!(stitcher.image().data(), truth.data(), "往上的内容没有拼到正确位置");
    }

    #[test]
    fn reverse_horizontal_scroll_is_detected_and_prepended() {
        let big = content(500, 120);
        let xs = [200u32, 150, 100, 50];
        let frames: Vec<Frame> = xs.iter().map(|x| window_of(&big, *x, 0, 100, 120)).collect();
        let mut stitcher = DirectedStitcher::new(frames[0].clone(), 4000, MatchOptions::default());
        for frame in &frames[1..] {
            stitcher.push(frame.clone());
        }
        assert_eq!(stitcher.axis(), Axis::Horizontal, "往回横滚也要能判出横向");
        assert_eq!(stitcher.size(), Size::new(250, 120));
        let truth = window_of(&big, 50, 0, 250, 120);
        assert_eq!(stitcher.image().data(), truth.data(), "往回横滚的内容顺序不对");
    }

    #[test]
    fn scrolling_down_then_back_up_covers_the_whole_range() {
        let big = content(120, 600);
        // 从中间开始，先往下滚两帧，再往回滚四帧（越过起点）。
        // ⚠️ 步长必须**小于视口高度**：一步正好一屏时两帧毫无重叠，位移根本估不出来
        // （真实用户也不会这么滚——那已经不是"滚动截屏"而是"翻页"了）。
        let ys = [300u32, 360, 420, 360, 300, 240, 180];
        let frames: Vec<Frame> = ys.iter().map(|y| window_of(&big, 0, *y, 120, 100)).collect();
        let mut stitcher = DirectedStitcher::new(frames[0].clone(), 4000, MatchOptions::default());
        for frame in &frames[1..] {
            stitcher.push(frame.clone());
        }
        assert_eq!(stitcher.size(), Size::new(120, 340), "来回滚之后应当是 180..520");
        let truth = window_of(&big, 0, 180, 120, 340);
        assert_eq!(stitcher.image().data(), truth.data(), "来回滚的内容顺序不对");
    }}
