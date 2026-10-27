//! 框选区域时的边缘吸附：选区的边进入吸附距离后，对齐到附近窗口或控件的边缘。
//!
//! 吸附目标是拍冻结帧那一刻的窗口布局（[`WindowLayout`]）里各窗口的外框（与用户看到的冻结帧
//! 一致），以及选区那一角所在窗口里控件的外框。控件按需取：指针第一次进入某个窗口时，
//! 由持有平台的一方在把输入交给选区状态机之前调用 [`Snapping::load_controls`] 补上。
//! 被上层窗口盖住的边不算。
//! 按住 Ctrl 时临时不吸附：拖拽起点看按下左键时，拖动的一角看最近一次指针输入时。
//!
//! 坐标约定：“边”是像素之间的分界线。竖直的边 `x = L` 位于第 `L - 1` 列与第 `L` 列像素之间，
//! 所以外框 `Rect { x, width, .. }` 的左右两条边是 `x` 和 `x + width`；选区的边也这样表示，
//! 选区右边对齐窗口右边时，选区最右一列像素正是窗口最右一列像素。

use std::collections::HashMap;

use bps_platform::{
    Modifiers, OverlayInput, Point, PointerButton, Rect, WindowId, WindowLayout, WindowSource,
};

/// 100% 缩放下的吸附距离（物理像素），按选区那一端所在显示器的缩放比例放大。
pub(crate) const SNAP_DISTANCE: f64 = 8.0;

/// 按住这些修饰键时临时不吸附。
fn suspends_snapping(modifiers: Modifiers) -> bool {
    modifiers.ctrl
}

/// 框选时的吸附：吸附目标（窗口和控件的外框），以及拖拽两端此刻是否吸附。
#[derive(Debug, Default)]
pub(crate) struct Snapping {
    layout: WindowLayout,
    /// 已经取过的各窗口里控件的外框。
    controls: HashMap<WindowId, Vec<Rect>>,
    /// 拖拽起点按下时按住了 Ctrl。
    anchor_suspended: bool,
    /// 最近一次指针输入时按住了 Ctrl。
    current_suspended: bool,
}

/// 选区拖拽的一端：拖拽起点或当前指针位置（已限制在虚拟桌面内）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DragEnd {
    point: Point,
    /// 这一端在多远（物理像素）以内吸附。
    distance: i32,
    /// 这一端此刻是否吸附（没有按住 Ctrl）。
    snaps: bool,
}

impl DragEnd {
    /// 位于 `point`、在 `distance` 物理像素以内吸附的一端。
    pub fn new(point: Point, distance: i32) -> Self {
        Self {
            point,
            distance,
            snaps: true,
        }
    }
}

impl Snapping {
    /// 吸附到此刻桌面上窗口的边缘：在拍冻结帧（或打开实时遮罩）之前调用，
    /// 使吸附目标与用户看到的画面一致。取不到窗口布局时不吸附。
    pub fn of_desktop(windows: &dyn WindowSource) -> Self {
        Self {
            layout: windows.layout().unwrap_or_default(),
            ..Self::default()
        }
    }

    /// `input` 的指针位置在某个窗口上、还没取过它的控件时，向 `windows` 取来；取不到就当没有控件。
    pub fn load_controls(&mut self, input: &OverlayInput, windows: &dyn WindowSource) {
        let position = match *input {
            OverlayInput::PointerMoved { position, .. }
            | OverlayInput::PointerPressed { position, .. }
            | OverlayInput::PointerReleased { position, .. } => position,
            OverlayInput::KeyPressed { .. }
            | OverlayInput::Character(_)
            | OverlayInput::Interrupted => return,
        };
        if let Some(window) = self.layout.window_at(position)
            && !self.controls.contains_key(&window.id)
        {
            let controls = windows.controls(window.id).unwrap_or_default();
            self.controls.insert(window.id, controls);
        }
    }

    /// 记下用户输入时按住的修饰键：按下左键开始一次拖拽，两端都看此刻；之后只更新拖动的一端。
    pub fn observe(&mut self, input: &OverlayInput) {
        match *input {
            OverlayInput::PointerPressed {
                button: PointerButton::Left,
                modifiers,
                ..
            } => {
                self.anchor_suspended = suspends_snapping(modifiers);
                self.current_suspended = self.anchor_suspended;
            }
            OverlayInput::PointerMoved { modifiers, .. }
            | OverlayInput::PointerReleased { modifiers, .. } => {
                self.current_suspended = suspends_snapping(modifiers);
            }
            _ => {}
        }
    }

    /// 以 `anchor`、`current` 两个像素为对角的选区，各边吸附到附近窗口的边缘；
    /// 结果不超出虚拟桌面 `bounds`。
    pub fn region(&self, bounds: Rect, mut anchor: DragEnd, mut current: DragEnd) -> Rect {
        anchor.snaps = !self.anchor_suspended;
        current.snaps = !self.current_suspended;
        let (left, right) = self.snap_axis(Axis::X, bounds, anchor, current);
        let (top, bottom) = self.snap_axis(Axis::Y, bounds, anchor, current);
        Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
    }

    /// 选区在 `axis` 方向上的两条边（前边、后边），各自吸附。
    fn snap_axis(&self, axis: Axis, bounds: Rect, anchor: DragEnd, current: DragEnd) -> (i32, i32) {
        let (low_end, high_end) = if axis.along(current.point) >= axis.along(anchor.point) {
            (anchor, current)
        } else {
            (current, anchor)
        };
        // 较小一端像素的前沿、较大一端像素的后沿。
        let low = axis.along(low_end.point);
        let high = axis.along(high_end.point) + 1;
        let snapped_low = self.snap(axis, bounds, low, low_end);
        let snapped_high = self.snap(axis, bounds, high, high_end);
        if snapped_low < snapped_high {
            (snapped_low, snapped_high)
        } else {
            (low, high)
        }
    }

    /// 把 `end` 那一端的边 `line` 吸附到距离以内最近的目标边缘；没有时原样返回。
    /// 目标是所有窗口的外框，以及这一角所在窗口里的控件；被上层窗口盖住的边不算。
    fn snap(&self, axis: Axis, bounds: Rect, line: i32, end: DragEnd) -> i32 {
        if !end.snaps {
            return line;
        }
        let (first, last) = axis.span(bounds);
        let windows = self
            .layout
            .windows()
            .iter()
            .map(|window| (window.id, window.bounds));
        let controls = self
            .layout
            .window_at(end.point)
            .and_then(|window| {
                let controls = self.controls.get(&window.id)?;
                Some(controls.iter().map(|control| (window.id, *control)))
            })
            .into_iter()
            .flatten();
        windows
            .chain(controls)
            .flat_map(|(owner, rect)| {
                edges_near(axis, rect, end).map(move |(edge, pixel)| (owner, edge, pixel))
            })
            .filter(|&(_, edge, _)| (first..=last).contains(&edge))
            .filter(|&(_, edge, _)| (edge - line).abs() <= end.distance)
            .filter(|&(owner, _, pixel)| self.layout.window_at(pixel).map(|w| w.id) == Some(owner))
            .map(|(_, edge, _)| edge)
            .min_by_key(|edge| (edge - line).abs())
            .unwrap_or(line)
    }
}

/// 外框 `rect` 在 `axis` 方向上、`end` 那一角附近的边，以及边上离这一角最近的像素
/// （判断这条边有没有被上层窗口盖住用）。
///
/// “附近”：沿边的方向上，这一角离那条边的两端不超过吸附距离。
fn edges_near(axis: Axis, rect: Rect, end: DragEnd) -> impl Iterator<Item = (i32, Point)> {
    let (start, stop) = axis.span(rect);
    let (cross_start, cross_stop) = axis.cross_span(rect);
    let across = axis.across(end.point);
    let near = rect.width > 0
        && rect.height > 0
        && cross_start - end.distance <= across
        && across < cross_stop + end.distance;
    let across = across.clamp(cross_start, (cross_stop - 1).max(cross_start));
    // 前边上的像素在边之后，后边上的像素在边之前。
    [(start, start), (stop, stop - 1)]
        .into_iter()
        .filter(move |_| near)
        .map(move |(edge, pixel)| (edge, axis.point(pixel, across)))
}

/// 沿哪个坐标轴：`X` 处理左右两条（竖直的）边，`Y` 处理上下两条（水平的）边。
#[derive(Debug, Clone, Copy)]
enum Axis {
    X,
    Y,
}

impl Axis {
    /// 点在这个轴上的坐标。
    fn along(self, point: Point) -> i32 {
        match self {
            Axis::X => point.x,
            Axis::Y => point.y,
        }
    }

    /// 这个轴上坐标为 `along`、另一个轴上坐标为 `across` 的点。
    fn point(self, along: i32, across: i32) -> Point {
        match self {
            Axis::X => Point::new(along, across),
            Axis::Y => Point::new(across, along),
        }
    }

    /// 点在另一个轴上的坐标。
    fn across(self, point: Point) -> i32 {
        self.other().along(point)
    }

    /// 矩形在这个轴上的前后两条边。
    fn span(self, rect: Rect) -> (i32, i32) {
        match self {
            Axis::X => (rect.x, rect.x + rect.width as i32),
            Axis::Y => (rect.y, rect.y + rect.height as i32),
        }
    }

    /// 矩形在另一个轴上的前后两条边。
    fn cross_span(self, rect: Rect) -> (i32, i32) {
        self.other().span(rect)
    }

    fn other(self) -> Axis {
        match self {
            Axis::X => Axis::Y,
            Axis::Y => Axis::X,
        }
    }
}
