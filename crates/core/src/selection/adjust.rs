//! 调整已框好的选区：拖动边框（手柄）改变大小、在选区内拖动移动它、用方向键逐像素微调。
//!
//! 选区的边用它包含的像素表示（左边 = 最左一列像素，右边 = 最右一列像素），拖过对边时
//! 由 [`between`] 自然翻转。所有调整都限制在虚拟桌面范围内，选区至少 1 × 1 像素。

use bps_platform::{Color, Modifiers, OverlayCursor, OverlayItem, OverlayKey, Point, Rect};

use super::{ACCENT, between};

/// 手柄的边长（100% 缩放下的物理像素，按所在显示器的缩放比例放大）。
const HANDLE: f64 = 7.0;
/// 手柄外圈的颜色，让手柄在深色、浅色画面上都看得清。
const HANDLE_RING: Color = Color::rgba(255, 255, 255, 255);

/// 在已框好的选区上按下左键时抓住的部位。两个方向都没有抓住边时是抓住整个选区（移动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Grip {
    /// 抓住了左边（`Start`）或右边（`End`）。
    x: Option<End>,
    /// 抓住了上边（`Start`）或下边（`End`）。
    y: Option<End>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Start,
    End,
}

impl Grip {
    /// 抓住的是整个选区：拖动时移动选区。
    pub fn is_move(self) -> bool {
        self.x.is_none() && self.y.is_none()
    }

    /// 指针在这个部位上时的形状。
    pub fn cursor(self) -> OverlayCursor {
        match (self.x, self.y) {
            (None, None) => OverlayCursor::Move,
            (Some(_), None) => OverlayCursor::ResizeEw,
            (None, Some(_)) => OverlayCursor::ResizeNs,
            (Some(x), Some(y)) if x == y => OverlayCursor::ResizeNwse,
            (Some(_), Some(_)) => OverlayCursor::ResizeNesw,
        }
    }
}

/// `point` 处抓住的部位：离某条边（边框线）不超过 `reach` 像素就抓住这条边，靠近角时同时
/// 抓住两条边；选区内其余地方抓住整个选区。离选区更远时为 `None`（在那里按下是重新框选）。
pub(super) fn grip_at(region: Rect, point: Point, reach: i32) -> Option<Grip> {
    let (left, right, top, bottom) = edges(region);
    let within = |v: i32, low: i32, high: i32| v >= low - reach && v <= high + reach;
    if !within(point.x, left, right) || !within(point.y, top, bottom) {
        return None;
    }
    // 边框线位于两列（行）像素之间，用两倍坐标比较距离。
    let axis = |v: i32, low: i32, high: i32| {
        let to_start = (2 * v - (2 * low - 1)).abs();
        let to_end = (2 * v - (2 * high + 1)).abs();
        let near = 2 * reach;
        match (to_start <= near, to_end <= near) {
            (true, true) if to_start <= to_end => Some(End::Start),
            (true, true) => Some(End::End),
            (true, false) => Some(End::Start),
            (false, true) => Some(End::End),
            (false, false) => None,
        }
    };
    let grip = Grip {
        x: axis(point.x, left, right),
        y: axis(point.y, top, bottom),
    };
    (!grip.is_move() || region.contains(point)).then_some(grip)
}

/// 抓住 `grip` 把选区 `region` 从 `from` 拖到 `to` 后的选区，限制在 `bounds` 内。
///
/// 移动时选区平移指针的位移；调整大小时被拖动的边跟着指针走（见 [`follow`]）。
pub(super) fn dragged(region: Rect, grip: Grip, from: Point, to: Point, bounds: Rect) -> Rect {
    if grip.is_move() {
        return shifted(
            region,
            grip,
            Point::new(to.x - from.x, to.y - from.y),
            bounds,
        );
    }
    let (low_x, high_x, low_y, high_y) = edges(bounds);
    let (mut left, mut right, mut top, mut bottom) = edges(region);
    let x = |edge: i32| follow(edge, from.x, to.x, (low_x, high_x));
    let y = |edge: i32| follow(edge, from.y, to.y, (low_y, high_y));
    match grip.x {
        Some(End::Start) => left = x(left),
        Some(End::End) => right = x(right),
        None => {}
    }
    match grip.y {
        Some(End::Start) => top = y(top),
        Some(End::End) => bottom = y(bottom),
        None => {}
    }
    between(Point::new(left, top), Point::new(right, bottom))
}

/// 抓住 `grip` 把选区从 `from` 拖到 `to` 时被拖动的那个像素（放大镜对准它）：被拖动的边
/// 所在的列（行），没有拖动的方向取指针的位置。移动整个选区时为 `None`。
pub(super) fn dragged_pixel(
    region: Rect,
    grip: Grip,
    from: Point,
    to: Point,
    bounds: Rect,
) -> Option<Point> {
    if grip.is_move() {
        return None;
    }
    let (left, right, top, bottom) = edges(region);
    let (low_x, high_x, low_y, high_y) = edges(bounds);
    let x = match grip.x {
        Some(End::Start) => follow(left, from.x, to.x, (low_x, high_x)),
        Some(End::End) => follow(right, from.x, to.x, (low_x, high_x)),
        None => to.x,
    };
    let y = match grip.y {
        Some(End::Start) => follow(top, from.y, to.y, (low_y, high_y)),
        Some(End::End) => follow(bottom, from.y, to.y, (low_y, high_y)),
        None => to.y,
    };
    Some(Point::new(x, y))
}

/// 被拖动的边（像素坐标 `edge`）在指针从 `from` 移到 `to` 后的位置：保持按下时指针与这条边
/// 的距离，限制在 `[low, high]` 内。指针已到虚拟桌面边缘（再也拖不出去）时边也贴到边缘，
/// 否则在边框外侧按下的手柄永远拖不到最边上的像素。指针在这个方向上没动时边不动。
fn follow(edge: i32, from: i32, to: i32, (low, high): (i32, i32)) -> i32 {
    if to == from {
        edge
    } else if to <= low {
        low
    } else if to >= high {
        high
    } else {
        (edge + to - from).clamp(low, high)
    }
}

/// 抓住 `grip` 把选区 `region` 挪动 `delta`：移动时平移整个选区，否则只挪抓住的边；
/// 限制在 `bounds` 内。
fn shifted(region: Rect, grip: Grip, delta: Point, bounds: Rect) -> Rect {
    let (low_x, high_x, low_y, high_y) = edges(bounds);
    if grip.is_move() {
        let fit = |v: i32, low: i32, high: i32, size: u32| v.min(high + 1 - size as i32).max(low);
        return Rect::new(
            fit(region.x + delta.x, low_x, high_x, region.width),
            fit(region.y + delta.y, low_y, high_y, region.height),
            region.width,
            region.height,
        );
    }
    let (left, right, top, bottom) = edges(region);
    let (left, right) = moved(grip.x, (left, right), delta.x, (low_x, high_x));
    let (top, bottom) = moved(grip.y, (top, bottom), delta.y, (low_y, high_y));
    between(Point::new(left, top), Point::new(right, bottom))
}

/// 用方向键微调选区：只按方向键时把选区移动 1 像素；按住 Ctrl 时朝这个方向的那条边向外
/// 移动 1 像素（扩大），按住 Shift 时背向这个方向的那条边向内移动 1 像素（缩小）。也就是说，
/// 被移动的边总是朝方向键所指的方向移动。限制在 `bounds` 内，选区至少 1 像素宽高。
/// 不是方向键或按着 Alt / Win 时返回 `None`。
pub(super) fn nudged(
    region: Rect,
    key: OverlayKey,
    modifiers: Modifiers,
    bounds: Rect,
) -> Option<Rect> {
    if modifiers.alt || modifiers.win {
        return None;
    }
    let (dx, dy) = match key {
        OverlayKey::Left => (-1, 0),
        OverlayKey::Right => (1, 0),
        OverlayKey::Up => (0, -1),
        OverlayKey::Down => (0, 1),
        _ => return None,
    };
    let delta = Point::new(dx, dy);
    let end = |d: i32| match d.signum() {
        -1 => Some(End::Start),
        1 => Some(End::End),
        _ => None,
    };
    let grip = if modifiers.ctrl {
        // 朝方向键的那条边：← 左边、→ 右边、↑ 上边、↓ 下边。
        Grip {
            x: end(dx),
            y: end(dy),
        }
    } else if modifiers.shift {
        // 背向方向键的那条边：← 右边、→ 左边、↑ 下边、↓ 上边。
        let (left, right, top, bottom) = edges(region);
        if (dx != 0 && left == right) || (dy != 0 && top == bottom) {
            return Some(region);
        }
        Grip {
            x: end(-dx),
            y: end(-dy),
        }
    } else {
        Grip { x: None, y: None }
    };
    Some(shifted(region, grip, delta, bounds))
}

/// 选区边框上的手柄：四个角和（边足够长时）四条边的中点，各是一个白圈蓝芯的小方块，
/// 中心在边框线上。`frame` 是边框宽度，`scale` 是所在显示器的缩放比例。
pub(super) fn handles(region: Rect, frame: u32, scale: f64) -> Vec<OverlayItem> {
    let size = ((HANDLE * scale).round() as i32).max(3);
    let ring = (scale.round() as i32).max(1);
    let frame = frame as i32;
    let start = |band: i32| band + (frame - size).div_euclid(2);
    // 每个方向三处：开始一边的边框、中点、结束一边的边框；中点只在边足够长时才放手柄。
    let positions = |origin: i32, length: u32| {
        let length = length as i32;
        let middle = (length > 3 * size).then(|| start(origin + (length - frame) / 2));
        [
            Some(start(origin - frame)),
            middle,
            Some(start(origin + length)),
        ]
    };
    let xs = positions(region.x, region.width);
    let ys = positions(region.y, region.height);
    let mut items = Vec::new();
    for (i, y) in ys.iter().enumerate() {
        for (j, x) in xs.iter().enumerate() {
            let (Some(x), Some(y)) = (*x, *y) else {
                continue;
            };
            if i == 1 && j == 1 {
                continue;
            }
            items.push(OverlayItem::Fill {
                rect: Rect::new(x, y, size as u32, size as u32),
                color: HANDLE_RING,
            });
            items.push(OverlayItem::Fill {
                rect: Rect::new(
                    x + ring,
                    y + ring,
                    (size - 2 * ring).max(1) as u32,
                    (size - 2 * ring).max(1) as u32,
                ),
                color: ACCENT,
            });
        }
    }
    items
}

/// 把一条边（`Start` 是开始一边，`End` 是结束一边）移动 `d`，限制在 `[low, high]` 内。
fn moved(
    end: Option<End>,
    (start, stop): (i32, i32),
    d: i32,
    (low, high): (i32, i32),
) -> (i32, i32) {
    match end {
        Some(End::Start) => ((start + d).clamp(low, high), stop),
        Some(End::End) => (start, (stop + d).clamp(low, high)),
        None => (start, stop),
    }
}

/// 矩形包含的最左、最右一列和最上、最下一行像素。
fn edges(rect: Rect) -> (i32, i32, i32, i32) {
    (
        rect.x,
        rect.x + rect.width as i32 - 1,
        rect.y,
        rect.y + rect.height as i32 - 1,
    )
}
