/// 虚拟桌面坐标系中的一个点，单位是物理像素。
///
/// 原点是主显示器左上角，其他显示器可以位于负坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// 虚拟桌面坐标系中的矩形，单位是物理像素。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 是否包含 `point`：左、上边包含，右、下边不包含。
    pub fn contains(&self, point: Point) -> bool {
        let (px, py) = (i64::from(point.x), i64::from(point.y));
        let (x, y) = (i64::from(self.x), i64::from(self.y));
        px >= x && py >= y && px < x + i64::from(self.width) && py < y + i64::from(self.height)
    }

    /// 两个矩形的重叠部分；不重叠（包括只有边相接）时返回 `None`。
    pub fn intersection(&self, other: Rect) -> Option<Rect> {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (left < right && top < bottom).then(|| Rect::from_edges(left, top, right, bottom))
    }

    /// 同时包含两个矩形的最小矩形。
    pub fn union(&self, other: Rect) -> Rect {
        Rect::from_edges(
            self.left().min(other.left()),
            self.top().min(other.top()),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    fn left(&self) -> i64 {
        i64::from(self.x)
    }

    fn top(&self) -> i64 {
        i64::from(self.y)
    }

    fn right(&self) -> i64 {
        self.left() + i64::from(self.width)
    }

    fn bottom(&self) -> i64 {
        self.top() + i64::from(self.height)
    }

    /// 由四条边构造；超出坐标范围的部分被截断（真实桌面坐标远小于这个范围）。
    fn from_edges(left: i64, top: i64, right: i64, bottom: i64) -> Rect {
        let coordinate = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let length = |v: i64| v.clamp(0, i64::from(u32::MAX)) as u32;
        Rect::new(
            coordinate(left),
            coordinate(top),
            length(right - left),
            length(bottom - top),
        )
    }
}
