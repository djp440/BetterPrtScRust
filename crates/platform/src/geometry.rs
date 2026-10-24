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
}
