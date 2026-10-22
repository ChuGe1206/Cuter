//! 框选吸附：纯几何，无平台依赖。
//!
//! 截图工具好不好用，一大半取决于**能不能一次就把框选对齐到窗口边缘**。
//! 手抖一两个像素去裁一张窗口截图，几乎是每个截图工具用户的日常痛点。
//!
//! 这里刻意做成纯函数：
//!
//! * 平台相关的部分只有「把窗口矩形列出来」（`cuter_core::capture::WindowDetector`）；
//! * 吸附本身是坐标运算，可以完整无头测试——包括边界与退化情形。
//!
//! ## 参考线从哪来
//!
//! 每个矩形贡献**三条竖直参考线**（左、水平中心、右）与**三条水平参考线**
//! （上、垂直中心、下）。屏幕自身的四条边与两条中线也同样贡献。
//! 这样「对齐到另一个窗口的中线」「对齐到屏幕正中」都是免费的。
//!
//! ## 三种拖拽各自的吸附语义
//!
//! | 手势 | 吸附对象 |
//! |------|----------|
//! | 新建选区 | 只吸附**正在移动的那个角**（起点在按下时吸附一次） |
//! | 平移选区 | 左/中/右三条边分别找最近参考线，取位移最小的那条 |
//! | 拉伸选区 | 只吸附**正在被拖动的那几条边**，其余边不动 |

use egui::{Pos2, Rect, Vec2};

/// 吸附容差（像素）。1:1 物理像素下 8px 是"手抖也能吸住"的舒适值。
pub const DEFAULT_TOLERANCE: f32 = 8.0;

/// 水平/竖直参考线集合。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapTargets {
    /// 竖直参考线的 x 坐标。
    pub xs: Vec<f32>,
    /// 水平参考线的 y 坐标。
    pub ys: Vec<f32>,
}

impl SnapTargets {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// 由窗口矩形 + 屏幕矩形构造参考线。
    #[must_use]
    pub fn from_rects(rects: &[Rect], screen: Rect) -> Self {
        let mut targets = Self::empty();
        for rect in rects.iter().chain(std::iter::once(&screen)) {
            if rect.width() <= 0.0 || rect.height() <= 0.0 {
                continue;
            }
            targets.xs.push(rect.left());
            targets.xs.push(rect.center().x);
            targets.xs.push(rect.right());
            targets.ys.push(rect.top());
            targets.ys.push(rect.center().y);
            targets.ys.push(rect.bottom());
        }
        // 去掉重复线：多个窗口常常共享同一条边（例如平铺布局），
        // 重复线不会给出不同结果，只会让最近邻查找做无用功。
        targets.xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        targets.xs.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        targets.ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        targets.ys.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        targets
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.xs.is_empty() && self.ys.is_empty()
    }
}

/// 一次吸附的结果。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SnapOutcome {
    /// 需要施加的位移（未吸附时为 `Vec2::ZERO`）。
    pub delta: Vec2,
    /// 命中的竖直参考线（需要画出来给用户看）。
    pub line_x: Option<f32>,
    /// 命中的水平参考线。
    pub line_y: Option<f32>,
}

impl SnapOutcome {
    #[must_use]
    pub fn is_snapped(&self) -> bool {
        self.line_x.is_some() || self.line_y.is_some()
    }
}

/// 正在被拖动的边（拉伸手势用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MovingEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl MovingEdges {
    #[must_use]
    pub fn left(handle_moves_left: bool, handle_moves_right: bool, handle_moves_top: bool, handle_moves_bottom: bool) -> Self {
        Self {
            left: handle_moves_left,
            right: handle_moves_right,
            top: handle_moves_top,
            bottom: handle_moves_bottom,
        }
    }
}

/// 在候选线里找离 `value` 最近、且距离不超过 `tolerance` 的那条。
///
/// 返回 `(位移, 参考线坐标)`。距离相同时取**更小的坐标**，保证结果稳定可测。
#[must_use]
fn nearest(value: f32, lines: &[f32], tolerance: f32) -> Option<(f32, f32)> {
    if tolerance <= 0.0 || !value.is_finite() {
        return None;
    }
    let mut best: Option<(f32, f32)> = None;
    for &line in lines {
        if !line.is_finite() {
            continue;
        }
        let delta = line - value;
        if delta.abs() > tolerance {
            continue;
        }
        match best {
            None => best = Some((delta, line)),
            Some((best_delta, best_line)) => {
                let better = delta.abs() < best_delta.abs()
                    // 距离相同取坐标更小的，避免不同枚举顺序给出不同结果
                    || (delta.abs() == best_delta.abs() && line < best_line);
                if better {
                    best = Some((delta, line));
                }
            }
        }
    }
    best
}

/// 从若干候选里选出位移绝对值最小的那个。
fn best_of(candidates: impl IntoIterator<Item = (f32, f32)>) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for (delta, line) in candidates {
        match best {
            None => best = Some((delta, line)),
            Some((best_delta, best_line)) => {
                if delta.abs() < best_delta.abs()
                    || (delta.abs() == best_delta.abs() && line < best_line)
                {
                    best = Some((delta, line));
                }
            }
        }
    }
    best
}

/// 新建选区：吸附正在移动的那个角。
#[must_use]
pub fn snap_corner(current: Pos2, targets: &SnapTargets, tolerance: f32) -> SnapOutcome {
    let x = nearest(current.x, &targets.xs, tolerance);
    let y = nearest(current.y, &targets.ys, tolerance);
    SnapOutcome {
        delta: Vec2::new(x.map_or(0.0, |(d, _)| d), y.map_or(0.0, |(d, _)| d)),
        line_x: x.map(|(_, line)| line),
        line_y: y.map(|(_, line)| line),
    }
}

/// 平移整个选区：三条边各找最近的参考线，取位移最小的。
#[must_use]
pub fn snap_move(rect: Rect, targets: &SnapTargets, tolerance: f32) -> SnapOutcome {
    let x = best_of(
        [
            nearest(rect.left(), &targets.xs, tolerance),
            nearest(rect.center().x, &targets.xs, tolerance),
            nearest(rect.right(), &targets.xs, tolerance),
        ]
        .into_iter()
        .flatten(),
    );
    let y = best_of(
        [
            nearest(rect.top(), &targets.ys, tolerance),
            nearest(rect.center().y, &targets.ys, tolerance),
            nearest(rect.bottom(), &targets.ys, tolerance),
        ]
        .into_iter()
        .flatten(),
    );
    SnapOutcome {
        delta: Vec2::new(x.map_or(0.0, |(d, _)| d), y.map_or(0.0, |(d, _)| d)),
        line_x: x.map(|(_, line)| line),
        line_y: y.map(|(_, line)| line),
    }
}

/// 拉伸选区：只吸附正在被拖动的那几条边。
#[must_use]
pub fn snap_resize(
    rect: Rect,
    moving: MovingEdges,
    targets: &SnapTargets,
    tolerance: f32,
) -> SnapOutcome {
    let x = best_of(
        [
            moving.left.then(|| nearest(rect.left(), &targets.xs, tolerance)),
            moving.right.then(|| nearest(rect.right(), &targets.xs, tolerance)),
        ]
        .into_iter()
        .flatten()
        .flatten(),
    );
    let y = best_of(
        [
            moving.top.then(|| nearest(rect.top(), &targets.ys, tolerance)),
            moving.bottom.then(|| nearest(rect.bottom(), &targets.ys, tolerance)),
        ]
        .into_iter()
        .flatten()
        .flatten(),
    );
    SnapOutcome {
        delta: Vec2::new(x.map_or(0.0, |(d, _)| d), y.map_or(0.0, |(d, _)| d)),
        line_x: x.map(|(_, line)| line),
        line_y: y.map(|(_, line)| line),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1920.0, 1080.0))
    }

    fn window(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn targets_include_edges_centers_and_screen() {
        let targets = SnapTargets::from_rects(&[window(100.0, 200.0, 400.0, 300.0)], screen());
        // 窗口：左 100 / 中 300 / 右 500
        for expected in [100.0, 300.0, 500.0] {
            assert!(targets.xs.contains(&expected), "缺少竖直参考线 {expected}");
        }
        // 窗口：上 200 / 中 350 / 下 500
        for expected in [200.0, 350.0, 500.0] {
            assert!(targets.ys.contains(&expected), "缺少水平参考线 {expected}");
        }
        // 屏幕：左 0 / 中 960 / 右 1920
        for expected in [0.0, 960.0, 1920.0] {
            assert!(targets.xs.contains(&expected), "缺少屏幕参考线 {expected}");
        }
    }

    #[test]
    fn duplicate_lines_are_collapsed() {
        // 平铺布局：两个窗口共享同一条边
        let targets = SnapTargets::from_rects(
            &[window(0.0, 0.0, 960.0, 1080.0), window(960.0, 0.0, 960.0, 1080.0)],
            screen(),
        );
        let count = targets.xs.iter().filter(|x| (**x - 960.0).abs() < 0.01).count();
        assert_eq!(count, 1, "共享边应当只保留一条参考线");
    }

    #[test]
    fn degenerate_rects_contribute_nothing() {
        let targets = SnapTargets::from_rects(
            &[window(10.0, 10.0, 0.0, 100.0), window(10.0, 10.0, 100.0, 0.0)],
            Rect::NOTHING,
        );
        assert!(targets.is_empty());
    }

    #[test]
    fn nearest_picks_closest_within_tolerance() {
        let lines = [100.0, 200.0, 1000.0];
        assert_eq!(nearest(197.0, &lines, 8.0), Some((3.0, 200.0)));
        // 205 离 200 只有 5，在容差内 → 应当吸附
        assert_eq!(nearest(205.0, &lines, 8.0), Some((-5.0, 200.0)));
        // 215 离最近的一条（200）也有 15 → 超出容差，不吸附
        assert_eq!(nearest(215.0, &lines, 8.0), None, "超出容差不应吸附");
        assert_eq!(nearest(1000.0, &lines, 8.0), Some((0.0, 1000.0)));
    }

    #[test]
    fn nearest_is_stable_on_ties() {
        // 恰好夹在两条线中间：应稳定取坐标更小的那条
        let lines = [100.0, 104.0];
        assert_eq!(nearest(102.0, &lines, 8.0), Some((-2.0, 100.0)));
    }

    #[test]
    fn nearest_ignores_non_finite_inputs() {
        assert_eq!(nearest(f32::NAN, &[100.0], 8.0), None);
        assert_eq!(nearest(100.0, &[f32::NAN], 8.0), None);
        assert_eq!(nearest(100.0, &[100.0], 0.0), None, "零容差应当关闭吸附");
    }

    #[test]
    fn snap_corner_moves_only_the_moving_corner() {
        let targets = SnapTargets::from_rects(&[window(100.0, 200.0, 400.0, 300.0)], screen());
        let outcome = snap_corner(Pos2::new(497.0, 203.0), &targets, 8.0);
        assert_eq!(outcome.delta, Vec2::new(3.0, -3.0));
        assert_eq!(outcome.line_x, Some(500.0));
        assert_eq!(outcome.line_y, Some(200.0));
    }

    #[test]
    fn snap_corner_does_nothing_far_from_any_line() {
        let targets = SnapTargets::from_rects(&[window(100.0, 200.0, 400.0, 300.0)], screen());
        let outcome = snap_corner(Pos2::new(237.0, 837.0), &targets, 8.0);
        assert_eq!(outcome, SnapOutcome::default());
        assert!(!outcome.is_snapped());
    }

    #[test]
    fn snap_move_prefers_the_smallest_edge_shift() {
        let targets = SnapTargets::from_rects(&[window(300.0, 300.0, 400.0, 400.0)], screen());
        // 选区 305..505：左边界离窗口左 300 差 5，右边界离窗口右 700 差 195
        let rect = window(305.0, 305.0, 200.0, 200.0);
        let outcome = snap_move(rect, &targets, 8.0);
        assert_eq!(outcome.delta.x, -5.0, "应当吸附左边界而不是右边界");
        assert_eq!(outcome.line_x, Some(300.0));
    }

    #[test]
    fn snap_move_can_align_centers() {
        let targets = SnapTargets::from_rects(&[window(300.0, 0.0, 400.0, 100.0)], screen());
        // 选区中心 502，窗口中心 500
        let rect = window(452.0, 200.0, 100.0, 100.0);
        let outcome = snap_move(rect, &targets, 8.0);
        assert_eq!(outcome.delta.x, -2.0);
        assert_eq!(outcome.line_x, Some(500.0));
    }

    #[test]
    fn snap_resize_only_touches_moving_edges() {
        let targets = SnapTargets::from_rects(&[window(100.0, 100.0, 400.0, 300.0)], screen());
        let rect = window(98.0, 97.0, 400.0, 300.0);
        // 只拖左边：右边即使离参考线很近（498 vs 500）也不应被吸附
        let outcome = snap_resize(
            rect,
            MovingEdges { left: true, right: false, top: false, bottom: false },
            &targets,
            8.0,
        );
        assert_eq!(outcome.delta.x, 2.0);
        assert_eq!(outcome.delta.y, 0.0, "竖直方向没有边在动，不应吸附");
    }

    #[test]
    fn snap_resize_with_no_moving_edges_is_a_noop() {
        let targets = SnapTargets::from_rects(&[window(100.0, 100.0, 400.0, 300.0)], screen());
        let outcome = snap_resize(window(99.0, 99.0, 10.0, 10.0), MovingEdges::default(), &targets, 8.0);
        assert_eq!(outcome, SnapOutcome::default());
    }

    #[test]
    fn empty_targets_never_snap() {
        let targets = SnapTargets::empty();
        assert!(!snap_corner(Pos2::new(1.0, 2.0), &targets, 8.0).is_snapped());
        assert!(!snap_move(window(1.0, 2.0, 3.0, 4.0), &targets, 8.0).is_snapped());
        assert!(
            !snap_resize(
                window(1.0, 2.0, 3.0, 4.0),
                MovingEdges { left: true, right: true, top: true, bottom: true },
                &targets,
                8.0
            )
            .is_snapped()
        );
    }

    #[test]
    fn negative_origin_screen_works() {
        // 多显示器：虚拟桌面原点在负半轴
        let screen = Rect::from_min_size(Pos2::new(-1920.0, 0.0), Vec2::new(3840.0, 1080.0));
        let targets = SnapTargets::from_rects(&[window(-500.0, 100.0, 200.0, 200.0)], screen);
        let outcome = snap_corner(Pos2::new(-303.0, 302.0), &targets, 8.0);
        assert_eq!(outcome.line_x, Some(-300.0));
        assert_eq!(outcome.line_y, Some(300.0));
    }
}
