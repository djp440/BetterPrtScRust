//! 选区遮罩上框选区域的交互状态机，以及每个状态下遮罩要画的场景。
//!
//! 平台无关：输入是平台递交的 [`OverlayInput`]，输出是下一步（继续 / 确定某区域 / 取消）
//! 和描述遮罩画面的 [`OverlayScene`]。坐标一律是虚拟桌面物理像素。
//!
//! 交互规则：
//! - 打开了 [`RegionSelection::pick_windows`] 时，没有选区时指针下的顶层窗口高亮（它以外变暗、
//!   内侧描边、标出窗口尺寸）。单击（按下后拖动不超过拖拽阈值就松开，按下和松开在同一个窗口上）
//!   或没有选区时按 Enter，选中这个窗口：选区变成窗口在虚拟桌面内的部分，并返回
//!   [`Step::PickedWindow`]；之后调整选区就不再是“这个窗口”（[`RegionSelection::picked_window`]）。
//! - 按住左键拖动超过拖拽阈值开始框选；松开后选区确定下来（`Selected`），可以在选区外重新拖拽替换它。
//! - 选区确定后按 Enter 或双击选区内部，完成框选。例外：打开了
//!   [`RegionSelection::confirm_on_release`]（截后动作为复制到剪贴板，或直达 OCR）时，拖拽框出有效选区、
//!   松开左键的那一刻就完成，不显示手柄，也不需要 Enter 或双击；拖拽过程中 Esc、右键取消、
//!   放大镜、按 C 取色、边缘吸附照常。
//! - Esc 随时取消整次捕获；右键在有选区（或正在拖拽）时清除选区，没有选区时取消捕获。
//! - 选区包含拖拽起点和终点两个像素；按下后没有拖过拖拽阈值就松开不算框选。
//! - 选区框好后边框上有手柄：在边框附近按下拖动改变大小（靠近角时同时改两条边，拖过对边时
//!   翻转），在选区内按下拖动移动选区；方向键逐像素微调（见 `adjust::nudged`）。都限制在
//!   虚拟桌面内。
//! - 还没有选区、拖拽框选、拖动手柄时显示放大镜（[`RegionSelection::magnifier_focus`]），
//!   这时按 C 复制放大镜对准的像素的颜色值（[`Step::CopyColor`]）。
//! - 选区锁定（[`RegionSelection::lock`]，开始标注）后仍显示边框手柄，隐藏放大镜。
//! - 拖拽时选区的边吸附到附近窗口、控件的边缘，按住 Ctrl 临时不吸附（见 [`crate::snap`]）。
//!
//! 扩展点（后续 ticket）：在 [`State`] 里加状态或字段，在 [`RegionSelection::handle`] 里处理
//! 新输入，在 [`RegionSelection::scene`] 里按层追加绘制元素。调整选区的几何计算在 `adjust`
//! 子模块；吸附（#11）可以在拖拽框选和 `adjust::dragged` 算出新边的那一步加入。

use bps_platform::{
    Color, Corner, Display, Label, OverlayCursor, OverlayInput, OverlayItem, OverlayKey,
    OverlayScene, Point, PointerButton, Rect, TopLevelWindow,
};

use crate::snap::{DragEnd, SNAP_DISTANCE, Snapping};
use crate::stitch::virtual_desktop_bounds;

mod adjust;

use adjust::Grip;

/// 选区外的半透明遮挡。
pub(crate) const MASK: Color = Color::rgba(0, 0, 0, 115);
/// 选区边框。
pub(crate) const ACCENT: Color = Color::rgba(30, 144, 255, 255);
pub(crate) const LABEL_TEXT: Color = Color::rgba(255, 255, 255, 255);
pub(crate) const LABEL_BACKGROUND: Color = Color::rgba(0, 0, 0, 180);
/// 以下尺寸是 100% 缩放下的值（物理像素），按标签所在显示器的缩放比例放大。
pub(crate) const FONT_SIZE: f32 = 13.0;
pub(crate) const LABEL_PADDING: f64 = 4.0;
pub(crate) const LABEL_GAP: f64 = 4.0;
/// 估算标签高度用的行高系数；只用于判断标签放在选区外还是选区内，实际尺寸由平台按字体决定。
const LINE_HEIGHT: f32 = 1.4;
/// 离边框多近（100% 缩放下的物理像素）按下就抓住这条边调整大小。
const GRIP_REACH: f64 = 6.0;

/// 框选的下一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// 还在框选。
    Continue,
    /// 用户确定了这个区域。
    Confirmed(Rect),
    /// 用户取消了这次捕获。
    Cancelled,
    /// 用户要复制放大镜对准的这个像素的颜色值，框选继续。
    CopyColor(Point),
    /// 用户单击选中了一个窗口（见 [`RegionSelection::picked_window`]），选区已是这个窗口。
    PickedWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 还没有选区，等待按下左键开始框选。
    Idle,
    /// 按下了左键，还没拖过拖拽阈值：松开算单击，拖过阈值转为框选。`previous` 同 `Dragging`。
    Pressed {
        anchor: Point,
        previous: Option<Rect>,
    },
    /// 按住左键拖拽中。`previous` 是拖拽开始前已有的选区，没拖出区域就松开时恢复它。
    Dragging {
        anchor: Point,
        current: Point,
        previous: Option<Rect>,
    },
    /// 已框出选区，等待确定或重新框选。
    Selected(Rect),
    /// 按住左键拖动手柄或选区调整已框好的选区。`original` 是按下时的选区，指针从 `from`
    /// 拖到了 `to`。
    Adjusting {
        original: Rect,
        grip: Grip,
        from: Point,
        to: Point,
    },
}

/// 一次区域框选的交互状态。
pub(crate) struct RegionSelection {
    displays: Vec<Display>,
    /// 虚拟桌面范围：指针位置和选区都限制在其中。
    bounds: Rect,
    state: State,
    /// 选区确定后的操作提示（已按界面语言翻译）；标注时由工具条代替，为 `None`。
    selected_hint: Option<String>,
    /// 指针最近的位置（限制在虚拟桌面内）；指针还没在遮罩上动过时为 `None`。
    pointer: Option<Point>,
    /// 选区已锁定（开始标注），不显示放大镜，也不提示拖动内部移动。
    locked: bool,
    /// 拖拽框选时选区的边吸附到窗口、控件的边缘；默认没有目标，不吸附。
    snapping: Snapping,
    /// 拖拽框出有效选区、松开左键时直接完成（复制到剪贴板的区域模式）。
    confirm_on_release: bool,
    /// 单击选择窗口（窗口来自吸附用的窗口布局）。
    picks_windows: bool,
    /// 遮罩出现时的光标位置：指针还没在遮罩上动过时按它高亮窗口（放大镜仍等指针动了再出现）。
    initial_cursor: Option<Point>,
    /// 按下后拖动超过多少才算拖拽（100% 缩放下的物理像素，水平、垂直）；没超过就松开算单击。
    drag_threshold: (u32, u32),
    /// 单击选中的窗口和当时的选区；选区变了就不再是这个窗口。
    picked: Option<(Rect, TopLevelWindow)>,
}

impl RegionSelection {
    /// 在冻结帧覆盖的这些显示器上框选，选区确定后显示 `selected_hint`；没有显示器时返回 `None`。
    pub fn new(displays: Vec<Display>, selected_hint: String) -> Option<Self> {
        let bounds = virtual_desktop_bounds(&displays)?;
        Some(Self {
            displays,
            bounds,
            state: State::Idle,
            selected_hint: Some(selected_hint),
            pointer: None,
            locked: false,
            snapping: Snapping::default(),
            confirm_on_release: false,
            picks_windows: false,
            initial_cursor: None,
            drag_threshold: (0, 0),
            picked: None,
        })
    }

    /// 没有选区时高亮指针下的顶层窗口，单击选中它（窗口布局来自 [`Self::with_snapping`]）。
    /// `cursor` 是遮罩出现时的光标位置，一开始就按它高亮；`drag_threshold` 见字段说明。
    pub fn pick_windows(mut self, cursor: Option<Point>, drag_threshold: (u32, u32)) -> Self {
        self.picks_windows = true;
        self.drag_threshold = drag_threshold;
        self.initial_cursor = cursor.map(|c| self.clamp(c));
        self
    }

    /// 单击选中、而且选区之后没再改动过的窗口。
    pub fn picked_window(&self) -> Option<&TopLevelWindow> {
        match (&self.picked, self.state) {
            (Some((region, window)), State::Selected(selected)) if *region == selected => {
                Some(window)
            }
            _ => None,
        }
    }

    /// 把选区换成 `region`（例如窗口截图缩小后显示的范围）。
    pub fn set_selected(&mut self, region: Rect) {
        self.state = State::Selected(region);
    }

    /// 拖拽框出有效选区、松开左键的那一刻就完成（[`Step::Confirmed`]），不停留在等待确认的
    /// `Selected` 状态。没移动就松开仍不算框选。
    pub fn confirm_on_release(mut self) -> Self {
        self.confirm_on_release = true;
        self
    }

    /// 选区已锁定：隐藏放大镜，边框仍可调整；输入由捕获会话先区分手柄与标注。
    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// 拖拽框选时，选区的边按 `snapping` 吸附。
    pub fn with_snapping(mut self, snapping: Snapping) -> Self {
        self.snapping = snapping;
        self
    }

    /// 一开始就已框好 `region`（例如显示器模式下直接标注整块显示器）。
    pub fn with_selected(mut self, region: Rect) -> Self {
        self.state = State::Selected(region);
        self
    }

    /// 选区确定后不显示操作提示。
    pub fn without_selected_hint(mut self) -> Self {
        self.selected_hint = None;
        self
    }

    /// 框选时的吸附，供持有平台的一方补充吸附目标。
    pub fn snapping_mut(&mut self) -> &mut Snapping {
        &mut self.snapping
    }

    /// 已经框好、等待确定的选区；还没有选区或正在拖拽时为 `None`。
    pub fn selected(&self) -> Option<Rect> {
        match self.state {
            State::Selected(region) => Some(region),
            _ => None,
        }
    }

    pub fn resizing(&self) -> bool {
        matches!(self.state, State::Adjusting { .. })
    }

    pub fn resize_hit(&self, point: Point) -> bool {
        self.region()
            .and_then(|r| self.grip_at(r, point))
            .is_some_and(|g| !g.is_move())
    }

    /// 处理一次用户输入。
    pub fn handle(&mut self, input: &OverlayInput) -> Step {
        use OverlayInput::*;
        self.snapping.observe(input);
        if let PointerMoved { position, .. }
        | PointerPressed { position, .. }
        | PointerReleased { position, .. } = *input
        {
            self.pointer = Some(self.clamp(position));
        }
        if let KeyPressed {
            key: OverlayKey::C,
            modifiers,
        } = *input
            && !(modifiers.ctrl || modifiers.alt || modifiers.win)
            && let Some(focus) = self.magnifier_focus()
        {
            return Step::CopyColor(focus);
        }
        match (self.state, *input) {
            (_, Interrupted)
            | (
                _,
                KeyPressed {
                    key: OverlayKey::Escape,
                    ..
                },
            ) => return Step::Cancelled,
            (
                State::Selected(region),
                KeyPressed {
                    key: OverlayKey::Enter,
                    ..
                },
            ) => return Step::Confirmed(region),
            (
                State::Idle,
                KeyPressed {
                    key: OverlayKey::Enter,
                    ..
                },
            ) => {
                if let Some(window) = self.hovered_window().cloned() {
                    return self.pick(window);
                }
            }
            (
                State::Selected(region),
                PointerPressed {
                    button: PointerButton::Left,
                    position,
                    double_click: true,
                    ..
                },
            ) if region.contains(position) => return Step::Confirmed(region),
            // 在边框（手柄）附近按下调整大小，在选区内按下移动选区。
            (
                State::Selected(region),
                PointerPressed {
                    button: PointerButton::Left,
                    position,
                    ..
                },
            ) if self.grip_at(region, position).is_some() => {
                let grip = self.grip_at(region, position).expect("刚检查过");
                let from = self.clamp(position);
                self.state = State::Adjusting {
                    original: region,
                    grip,
                    from,
                    to: from,
                };
            }
            (
                State::Adjusting {
                    original,
                    grip,
                    from,
                    ..
                },
                PointerMoved { position, .. },
            ) => {
                self.state = State::Adjusting {
                    original,
                    grip,
                    from,
                    to: self.clamp(position),
                };
            }
            (
                State::Adjusting {
                    original,
                    grip,
                    from,
                    ..
                },
                PointerReleased {
                    button: PointerButton::Left,
                    position,
                    ..
                },
            ) => {
                let to = self.clamp(position);
                self.state = State::Selected(self.adjusted(original, grip, from, to));
            }
            (State::Selected(region), KeyPressed { key, modifiers }) => {
                if let Some(nudged) = adjust::nudged(region, key, modifiers, self.bounds) {
                    self.state = State::Selected(nudged);
                }
            }
            (
                State::Idle | State::Selected(_),
                PointerPressed {
                    button: PointerButton::Left,
                    position,
                    ..
                },
            ) => {
                self.state = State::Pressed {
                    anchor: self.clamp(position),
                    previous: self.region(),
                };
            }
            (State::Pressed { anchor, previous }, PointerMoved { position, .. }) => {
                let current = self.clamp(position);
                if self.beyond_drag_threshold(anchor, current) {
                    self.state = State::Dragging {
                        anchor,
                        current,
                        previous,
                    };
                }
            }
            (
                State::Pressed { anchor, previous },
                PointerReleased {
                    button: PointerButton::Left,
                    position,
                    ..
                },
            ) => {
                self.state = previous.map_or(State::Idle, State::Selected);
                let window = self.clicked_window(anchor, self.clamp(position)).cloned();
                if let Some(window) = window {
                    return self.pick(window);
                }
            }
            (
                State::Dragging {
                    anchor, previous, ..
                },
                PointerMoved { position, .. },
            ) => {
                self.state = State::Dragging {
                    anchor,
                    current: self.clamp(position),
                    previous,
                };
            }
            (
                State::Dragging {
                    anchor, previous, ..
                },
                PointerReleased {
                    button: PointerButton::Left,
                    position,
                    ..
                },
            ) => {
                let current = self.clamp(position);
                if current != anchor && self.confirm_on_release {
                    let region = self.dragged(anchor, current);
                    self.state = State::Selected(region);
                    return Step::Confirmed(region);
                }
                self.state = if current == anchor {
                    previous.map_or(State::Idle, State::Selected)
                } else {
                    State::Selected(self.dragged(anchor, current))
                };
            }
            (
                State::Idle,
                PointerReleased {
                    button: PointerButton::Right,
                    ..
                },
            ) => return Step::Cancelled,
            (
                State::Pressed { .. }
                | State::Dragging { .. }
                | State::Selected(_)
                | State::Adjusting { .. },
                PointerReleased {
                    button: PointerButton::Right,
                    ..
                },
            ) => self.state = State::Idle,
            _ => {}
        }
        Step::Continue
    }

    /// 当前的选区（拖拽、调整中则是正在拖出的区域）。
    pub fn region(&self) -> Option<Rect> {
        match self.state {
            State::Idle => None,
            State::Pressed { previous, .. } => previous,
            State::Dragging {
                anchor, current, ..
            } => Some(self.dragged(anchor, current)),
            State::Selected(region) => Some(region),
            State::Adjusting {
                original,
                grip,
                from,
                to,
            } => Some(self.adjusted(original, grip, from, to)),
        }
    }

    /// 放大镜对准的像素；不显示放大镜时为 `None`。还没有选区时对准指针，拖拽框选时对准
    /// 拖动的角，拖动手柄时对准被拖动的边；选区框好后、移动选区时和锁定后不显示。
    pub fn magnifier_focus(&self) -> Option<Point> {
        if self.locked {
            return None;
        }
        match self.state {
            State::Idle | State::Pressed { .. } => self.pointer,
            State::Dragging { current, .. } => Some(current),
            State::Selected(_) => None,
            State::Adjusting {
                original,
                grip,
                from,
                to,
            } => adjust::dragged_pixel(original, grip, from, to, self.bounds),
        }
    }

    /// `point` 处能抓住选区的哪个部位；抓取范围按指针所在显示器的缩放比例放大。
    fn grip_at(&self, region: Rect, point: Point) -> Option<Grip> {
        let reach = (GRIP_REACH * self.scale_at(point)).round() as i32;
        adjust::grip_at(region, point, reach)
    }

    /// 指针形状：在手柄上、拖动手柄时是调整大小的箭头，在选区内、移动选区时是移动箭头，
    /// 其余是十字。
    fn cursor(&self) -> OverlayCursor {
        let grip = match self.state {
            State::Adjusting { grip, .. } => Some(grip),
            State::Selected(region) => self.pointer.and_then(|p| self.grip_at(region, p)),
            State::Idle | State::Pressed { .. } | State::Dragging { .. } => None,
        };
        grip.filter(|g| !self.locked || !g.is_move())
            .map_or(OverlayCursor::Crosshair, Grip::cursor)
    }

    /// 选中 `window`：选区变成它在虚拟桌面内的部分。窗口完全在虚拟桌面外时什么也不做。
    fn pick(&mut self, window: TopLevelWindow) -> Step {
        let Some(region) = window.bounds.intersection(self.bounds) else {
            return Step::Continue;
        };
        self.state = State::Selected(region);
        self.picked = Some((region, window));
        Step::PickedWindow
    }

    /// `point` 处用户看到的顶层窗口；不选择窗口时为 `None`。
    fn window_at(&self, point: Point) -> Option<&TopLevelWindow> {
        if !self.picks_windows {
            return None;
        }
        self.snapping.layout().window_at(point)
    }

    /// 还没有选区时指针下高亮的窗口。
    fn hovered_window(&self) -> Option<&TopLevelWindow> {
        match self.state {
            _ if self.locked => None,
            State::Idle | State::Pressed { previous: None, .. } => self
                .pointer
                .or(self.initial_cursor)
                .and_then(|p| self.window_at(p)),
            _ => None,
        }
    }

    /// 在 `pressed` 按下、在 `released` 松开（没拖过阈值）时单击中的窗口：两处是同一个窗口才算。
    fn clicked_window(&self, pressed: Point, released: Point) -> Option<&TopLevelWindow> {
        let window = self.window_at(pressed)?;
        (self.window_at(released).map(|w| w.id) == Some(window.id)).then_some(window)
    }

    /// 从 `anchor` 移到 `current` 是否超过了拖拽阈值（按 `anchor` 所在显示器的缩放比例放大）。
    fn beyond_drag_threshold(&self, anchor: Point, current: Point) -> bool {
        let scale = self.scale_at(anchor);
        let limit = |base: u32| (f64::from(base) * scale).round() as i32;
        (current.x - anchor.x).abs() > limit(self.drag_threshold.0)
            || (current.y - anchor.y).abs() > limit(self.drag_threshold.1)
    }

    /// 高亮 `window`：它以外变暗、内侧描边、左上角标出窗口尺寸。窗口不在虚拟桌面内时返回 `false`。
    fn window_highlight(&self, window: &TopLevelWindow, items: &mut Vec<OverlayItem>) -> bool {
        let Some(visible) = window.bounds.intersection(self.bounds) else {
            return false;
        };
        items.extend(mask_around(self.bounds, visible));
        let corner = Point::new(visible.x, visible.y);
        let scale = self.scale_at(corner);
        let border = (scale.round() as u32).max(1);
        // 描边画在窗口内侧：最大化、全屏的窗口外侧没有地方可画。
        if let Some(inner) = inset(visible, border) {
            items.push(OverlayItem::Frame {
                rect: inner,
                width: border,
                color: ACCENT,
            });
        }
        let gap = (LABEL_GAP * scale).round() as i32 + border as i32;
        items.push(OverlayItem::Label(Label {
            text: format!("{} × {}", window.bounds.width, window.bounds.height),
            anchor: Point::new(corner.x + gap, corner.y + gap),
            corner: Corner::TopLeft,
            font_size: FONT_SIZE * scale as f32,
            padding: (LABEL_PADDING * scale).round() as u32,
            color: LABEL_TEXT,
            background: LABEL_BACKGROUND,
        }));
        true
    }

    /// 当前状态下遮罩上要画的内容，从下往上：遮挡、选区边框、标签。
    pub fn scene(&self) -> OverlayScene {
        let mut items = Vec::new();
        match self.region() {
            None => {
                let highlighted = self
                    .hovered_window()
                    .is_some_and(|window| self.window_highlight(window, &mut items));
                if !highlighted {
                    items.push(OverlayItem::Fill {
                        rect: self.bounds,
                        color: MASK,
                    });
                }
            }
            Some(region) => {
                items.extend(mask_around(self.bounds, region));
                let scale = self.scale_at(Point::new(region.x, region.y));
                let border = (scale.round() as u32).max(1);
                items.push(OverlayItem::Frame {
                    rect: region,
                    width: border,
                    color: ACCENT,
                });
                if matches!(self.state, State::Selected(_) | State::Adjusting { .. }) {
                    items.extend(adjust::handles(region, border, scale));
                }
                let size = format!("{} × {}", region.width, region.height);
                items.push(OverlayItem::Label(self.label_beside(
                    region,
                    Side::Above,
                    border,
                    size,
                )));
                if let Some(hint) = &self.selected_hint
                    && matches!(self.state, State::Selected(_))
                {
                    items.push(OverlayItem::Label(self.label_beside(
                        region,
                        Side::Below,
                        border,
                        hint.clone(),
                    )));
                }
            }
        }
        OverlayScene {
            items,
            cursor: self.cursor(),
            text_caret: None,
        }
    }

    /// 贴在选区上边外侧或下边外侧的标签；所在显示器容不下时改放在选区内侧。
    fn label_beside(&self, region: Rect, side: Side, border: u32, text: String) -> Label {
        let left = region.x;
        let top = region.y;
        let bottom = region.y + region.height as i32;
        let corner_pixel = match side {
            Side::Above => Point::new(left, top),
            Side::Below => Point::new(left, bottom - 1),
        };
        let display = self.display_at(corner_pixel).map(|d| d.bounds);
        let scale = self.scale_at(corner_pixel);
        let font_size = FONT_SIZE * scale as f32;
        let padding = (LABEL_PADDING * scale).round() as u32;
        let gap = (LABEL_GAP * scale).round() as i32;
        let height = (font_size * LINE_HEIGHT).ceil() as i32 + 2 * padding as i32;
        let outside = border as i32 + gap;
        let (anchor, corner) = match side {
            Side::Above => {
                let fits = display.is_none_or(|d| top - outside - height >= d.y);
                if fits {
                    (Point::new(left, top - outside), Corner::BottomLeft)
                } else {
                    (Point::new(left + gap, top + gap), Corner::TopLeft)
                }
            }
            Side::Below => {
                let fits =
                    display.is_none_or(|d| bottom + outside + height <= d.y + d.height as i32);
                if fits {
                    (Point::new(left, bottom + outside), Corner::TopLeft)
                } else {
                    (Point::new(left + gap, bottom - gap), Corner::BottomLeft)
                }
            }
        };
        Label {
            text,
            anchor,
            corner,
            font_size,
            padding,
            color: LABEL_TEXT,
            background: LABEL_BACKGROUND,
        }
    }

    /// 包含 `point` 的显示器。
    fn display_at(&self, point: Point) -> Option<&Display> {
        self.displays.iter().find(|d| d.bounds.contains(point))
    }

    /// `point` 所在显示器的缩放比例；不在任何显示器上时取主显示器的。
    fn scale_at(&self, point: Point) -> f64 {
        self.display_at(point)
            .or_else(|| self.displays.iter().find(|d| d.is_primary))
            .or_else(|| self.displays.first())
            .map_or(1.0, |d| d.scale_factor)
    }

    fn adjusted(&self, original: Rect, grip: Grip, from: Point, to: Point) -> Rect {
        let region = adjust::dragged(original, grip, from, to, self.bounds);
        if !self.locked || from == to {
            return region;
        }
        let Some(pixel) = adjust::dragged_pixel(original, grip, from, to, self.bounds) else {
            return region;
        };
        let end = DragEnd::new(pixel, (SNAP_DISTANCE * self.scale_at(to)).round() as i32);
        self.snapping.resized(self.bounds, region, end, grip.axes())
    }

    /// 从 `anchor` 拖到 `current` 框出的选区，边缘按吸附规则对齐。
    fn dragged(&self, anchor: Point, current: Point) -> Rect {
        let end = |point: Point| {
            DragEnd::new(point, (SNAP_DISTANCE * self.scale_at(point)).round() as i32)
        };
        self.snapping.region(self.bounds, end(anchor), end(current))
    }

    /// 把指针位置限制在虚拟桌面范围内。
    fn clamp(&self, point: Point) -> Point {
        let right = self.bounds.x + self.bounds.width as i32 - 1;
        let bottom = self.bounds.y + self.bounds.height as i32 - 1;
        Point::new(
            point.x.clamp(self.bounds.x, right),
            point.y.clamp(self.bounds.y, bottom),
        )
    }
}

#[derive(Clone, Copy)]
enum Side {
    Above,
    Below,
}

/// 四边各向内收 `by` 像素的矩形；收完没有剩余时返回 `None`。
fn inset(rect: Rect, by: u32) -> Option<Rect> {
    let width = rect.width.checked_sub(2 * by).filter(|w| *w > 0)?;
    let height = rect.height.checked_sub(2 * by).filter(|h| *h > 0)?;
    Some(Rect::new(
        rect.x + by as i32,
        rect.y + by as i32,
        width,
        height,
    ))
}

/// 以 `a`、`b` 两个像素为对角、同时包含它们的矩形。
pub(crate) fn between(a: Point, b: Point) -> Rect {
    let (left, right) = (a.x.min(b.x), a.x.max(b.x));
    let (top, bottom) = (a.y.min(b.y), a.y.max(b.y));
    Rect::new(
        left,
        top,
        (right - left) as u32 + 1,
        (bottom - top) as u32 + 1,
    )
}

/// 覆盖 `bounds` 中 `region` 以外部分的遮挡：上、下、左、右四条。
pub(crate) fn mask_around(bounds: Rect, region: Rect) -> Vec<OverlayItem> {
    let (left, top) = (bounds.x, bounds.y);
    let (right, bottom) = (left + bounds.width as i32, top + bounds.height as i32);
    let (r_left, r_top) = (region.x, region.y);
    let (r_right, r_bottom) = (r_left + region.width as i32, r_top + region.height as i32);
    let edges = [
        (left, top, right, r_top),
        (left, r_bottom, right, bottom),
        (left, r_top, r_left, r_bottom),
        (r_right, r_top, right, r_bottom),
    ];
    edges
        .into_iter()
        .filter(|&(l, t, r, b)| l < r && t < b)
        .map(|(l, t, r, b)| OverlayItem::Fill {
            rect: Rect::new(l, t, (r - l) as u32, (b - t) as u32),
            color: MASK,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bps_platform::{DisplayId, Modifiers};

    fn selection() -> RegionSelection {
        let display = Display {
            id: DisplayId(1),
            bounds: Rect::new(0, 0, 200, 150),
            scale_factor: 1.0,
            is_primary: true,
        };
        RegionSelection::new(vec![display], String::new()).unwrap()
    }

    fn press(x: i32, y: i32, double_click: bool) -> OverlayInput {
        OverlayInput::PointerPressed {
            button: PointerButton::Left,
            position: Point::new(x, y),
            modifiers: Modifiers::NONE,
            double_click,
        }
    }

    fn release(x: i32, y: i32) -> OverlayInput {
        OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: Point::new(x, y),
            modifiers: Modifiers::NONE,
        }
    }

    fn moved(x: i32, y: i32) -> OverlayInput {
        OverlayInput::PointerMoved {
            position: Point::new(x, y),
            modifiers: Modifiers::NONE,
        }
    }

    fn drag(selection: &mut RegionSelection, from: (i32, i32), to: (i32, i32)) -> Step {
        selection.handle(&press(from.0, from.1, false));
        selection.handle(&moved(to.0, to.1));
        selection.handle(&release(to.0, to.1))
    }

    #[test]
    fn double_click_inside_selection_completes_it() {
        let mut selection = selection();
        assert_eq!(drag(&mut selection, (10, 5), (29, 16)), Step::Continue);
        assert_eq!(
            selection.handle(&press(15, 10, true)),
            Step::Confirmed(Rect::new(10, 5, 20, 12))
        );
    }

    #[test]
    fn dragging_again_outside_selection_replaces_it_and_click_outside_keeps_it() {
        let mut selection = selection();
        drag(&mut selection, (10, 5), (29, 16));
        drag(&mut selection, (40, 30), (49, 39));
        selection.handle(&press(2, 2, false));
        selection.handle(&release(2, 2));
        assert_eq!(selection.selected(), Some(Rect::new(40, 30, 10, 10)));
    }

    #[test]
    fn confirm_on_release_completes_when_the_drag_is_released() {
        let mut selection = selection().confirm_on_release();
        assert_eq!(
            drag(&mut selection, (10, 5), (29, 16)),
            Step::Confirmed(Rect::new(10, 5, 20, 12))
        );
    }

    #[test]
    fn confirm_on_release_ignores_press_and_release_without_moving() {
        let mut selection = selection().confirm_on_release();
        selection.handle(&press(10, 5, false));
        assert_eq!(selection.handle(&release(10, 5)), Step::Continue);
        assert_eq!(selection.selected(), None);
    }
}
