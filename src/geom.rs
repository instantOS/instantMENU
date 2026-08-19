//! Geometry primitives shared by the menu core, the renderer and the
//! backends. All coordinates are root/window-space `i32` pixels; rectangles
//! with negative sizes are not meaningful (the C original allowed them in
//! places — the ports clamp or guard where it mattered).

/// A position in two-dimensional space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }

    /// Clamp both axes to be non-negative (the C `if (x < 0) x = 0` pairs).
    pub fn clamp_non_negative(self) -> Self {
        Point {
            x: self.x.max(0),
            y: self.y.max(0),
        }
    }
}

/// A width/height pair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Size { w, h }
    }
}

/// An axis-aligned rectangle: top-left corner plus size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// The zero-origin rectangle of the given size.
    pub const fn with_size(size: Size) -> Self {
        Rect { x: 0, y: 0, w: size.w, h: size.h }
    }

    pub fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(self) -> Size {
        Size::new(self.w, self.h)
    }

    /// Exclusive right edge.
    pub fn right(self) -> i32 {
        self.x + self.w
    }

    /// Exclusive bottom edge.
    pub fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Inclusive-bounds hit test — the `>= x && <= x + w` pattern the C
    /// version used for pointer targets.
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    /// Overlap area with `other` (dmenu's INTERSECT macro); 0 when disjoint.
    pub fn intersect_area(self, other: Rect) -> i32 {
        (0.max(self.right().min(other.right()) - self.x.max(other.x)))
            * (0.max(self.bottom().min(other.bottom()) - self.y.max(other.y)))
    }

    /// Linear interpolation towards `other` by factor `t` (0..1), as used by
    /// the selection/side-command animations.
    pub fn lerp(self, other: Rect, t: f64) -> Rect {
        fn axis(a: i32, b: i32, t: f64) -> i32 {
            (a as f64 + (b - a) as f64 * t) as i32
        }
        Rect::new(
            axis(self.x, other.x, t),
            axis(self.y, other.y, t),
            axis(self.w, other.w, t),
            axis(self.h, other.h, t),
        )
    }
}
