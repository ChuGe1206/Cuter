//! 物理像素几何类型。
//!
//! 刻意不使用 `egui::Pos2` / `Rect`：核心层必须与 GUI 解耦，且整型像素算术可以
//! 避免浮点误差在多显示器（含负坐标）场景下累积。

use serde::{Deserialize, Serialize};

/// 屏幕坐标点（物理像素，虚拟桌面坐标系；左上角显示器之外的点坐标为负）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0, y: 0 };

    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    /// 与另一点的欧氏距离。
    #[must_use]
    pub fn distance(self, other: Self) -> f32 {
        let dx = (self.x - other.x) as f32;
        let dy = (self.y - other.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }

    /// 四舍五入到最近的整数点（用于把 egui 的浮点坐标收回像素网格）。
    #[must_use]
    pub fn round(x: f32, y: f32) -> Self {
        Self::new(x.round() as i32, y.round() as i32)
    }
}

impl std::ops::Add for Point {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Point {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl std::ops::AddAssign for Point {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

/// 像素尺寸（宽高恒为非负）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

impl Size {
    pub const ZERO: Self = Self { w: 0, h: 0 };

    #[must_use]
    pub const fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }

    #[must_use]
    pub const fn area(self) -> u64 {
        self.w as u64 * self.h as u64
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// RGBA8 缓冲区所需字节数。
    #[must_use]
    pub const fn rgba_len(self) -> usize {
        self.w as usize * self.h as usize * 4
    }

    /// 等比缩放到不超过 `max` 以内（用于导出分辨率限制）。
    #[must_use]
    pub fn fit_within(self, max: Size) -> Self {
        if self.is_empty() || max.is_empty() {
            return self;
        }
        let scale = f64::min(
            max.w as f64 / self.w as f64,
            max.h as f64 / self.h as f64,
        );
        if scale >= 1.0 {
            return self;
        }
        Self::new(
            ((self.w as f64 * scale).round() as u32).max(1),
            ((self.h as f64 * scale).round() as u32).max(1),
        )
    }
}

/// 轴对齐矩形，`(x, y)` 为左上角（含），`(x+w, y+h)` 为右下角（不含）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const ZERO: Self = Self {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    #[must_use]
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// 以「左、上、右、下」边界构造；非法区间自动退化为空矩形。
    #[must_use]
    pub fn from_ltrb(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self::new(
            left,
            top,
            (right - left).max(0) as u32,
            (bottom - top).max(0) as u32,
        )
    }

    /// 由拖拽的任意两个角点构造规范化矩形（四个拖拽方向都能得到正确的 w/h）。
    #[must_use]
    pub fn from_corners(a: Point, b: Point) -> Self {
        Self::from_ltrb(
            a.x.min(b.x),
            a.y.min(b.y),
            a.x.max(b.x),
            a.y.max(b.y),
        )
    }

    #[must_use]
    pub const fn left(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn top(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn right(self) -> i32 {
        self.x + self.w as i32
    }

    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.y + self.h as i32
    }

    #[must_use]
    pub const fn size(self) -> Size {
        Size::new(self.w, self.h)
    }

    #[must_use]
    pub const fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }

    #[must_use]
    pub fn center(self) -> Point {
        Point::new(
            self.x + self.w as i32 / 2,
            self.y + self.h as i32 / 2,
        )
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    #[must_use]
    pub const fn area(self) -> u64 {
        self.w as u64 * self.h as u64
    }

    #[must_use]
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    #[must_use]
    pub fn contains_rect(self, other: Self) -> bool {
        !other.is_empty()
            && other.left() >= self.left()
            && other.top() >= self.top()
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.left() < other.right()
            && other.left() < self.right()
            && self.top() < other.bottom()
            && other.top() < self.bottom()
    }

    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            Self::ZERO
        } else {
            Self::from_ltrb(left, top, right, bottom)
        }
    }

    /// 同时覆盖两个矩形的最小矩形；空矩形被忽略。
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self::from_ltrb(
            self.left().min(other.left()),
            self.top().min(other.top()),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// 四边各向外扩张 `d` 像素（负值为收缩）。
    #[must_use]
    pub fn inflate(self, d: i32) -> Self {
        Self::from_ltrb(
            self.left() - d,
            self.top() - d,
            self.right() + d,
            self.bottom() + d,
        )
    }

    #[must_use]
    pub fn translate(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

}

impl std::ops::Add<Point> for Rect {
    type Output = Self;
    fn add(self, rhs: Point) -> Self {
        self.translate(rhs.x, rhs.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_corners_normalizes_all_drag_directions() {
        let expect = Rect::new(10, 20, 30, 40);
        let a = Point::new(10, 20);
        let b = Point::new(40, 60);
        assert_eq!(Rect::from_corners(a, b), expect);
        assert_eq!(Rect::from_corners(b, a), expect);
        assert_eq!(Rect::from_corners(Point::new(40, 20), Point::new(10, 60)), expect);
        assert_eq!(Rect::from_corners(Point::new(10, 60), Point::new(40, 20)), expect);
    }

    #[test]
    fn rect_bounds_are_half_open() {
        let r = Rect::new(-5, -5, 10, 10);
        assert!(r.contains(Point::new(-5, -5)));
        assert!(r.contains(Point::new(4, 4)));
        assert!(!r.contains(Point::new(5, 5)), "右下边界应为开区间");
        assert!(r.contains_rect(Rect::new(-5, -5, 10, 10)));
        assert!(!r.contains_rect(Rect::new(-5, -5, 11, 10)));
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(b), Rect::new(5, 5, 5, 5));
        assert_eq!(a.union(b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(Rect::new(100, 100, 5, 5)), Rect::ZERO);
        assert_eq!(Rect::ZERO.union(a), a);
    }

    #[test]
    fn intersect_touching_edges_is_empty() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(10, 0, 10, 10);
        assert!(!a.intersects(b));
        assert_eq!(a.intersect(b), Rect::ZERO);
    }

    #[test]
    fn clamp_via_intersect_keeps_region_inside_screen() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let sel = Rect::new(-50, -50, 200, 200);
        assert_eq!(sel.intersect(screen), Rect::new(0, 0, 150, 150));
    }

    #[test]
    fn size_fit_within_keeps_aspect() {
        let s = Size::new(4000, 2000);
        assert_eq!(s.fit_within(Size::new(1000, 1000)), Size::new(1000, 500));
        // 已经足够小则不放大
        assert_eq!(Size::new(100, 50).fit_within(Size::new(1000, 1000)), Size::new(100, 50));
    }
}
