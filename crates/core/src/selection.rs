//! 选区遮罩上框选区域的交互状态机，以及每个状态下遮罩要画的场景。
//!
//! 平台无关：输入是平台递交的 [`OverlayInput`]，输出是下一步（继续 / 确定某区域 / 取消）
//! 和描述遮罩画面的 [`OverlayScene`]。坐标一律是虚拟桌面物理像素。
//!
//! 交互规则：
//! - 按住左键拖拽框选；松开后选区确定下来（`Selected`），可以在选区外重新拖拽替换它。
//! - 选区确定后按 Enter 或双击选区内部，完成框选。
//! - Esc 随时取消整次捕获；右键在有选区（或正在拖拽）时清除选区，没有选区时取消捕获。
//! - 选区包含拖拽起点和终点两个像素；按下后没有移动就松开不算框选。
//! - 选区框好后边框上有手柄：在边框附近按下拖动改变大小（靠近角时同时改两条边，拖过对边时
//!   翻转），在选区内按下拖动移动选区；方向键逐像素微调（见 `adjust::nudged`）。都限制在
//!   虚拟桌面内。
//! - 还没有选区、拖拽框选、拖动手柄时显示放大镜（[`RegionSelection::magnifier_focus`]），
//!   这时按 C 复制放大镜对准的像素的颜色值（[`Step::CopyColor`]）。
//! - 选区锁定（[`RegionSelection::lock`]，开始标注）后不再显示手柄和放大镜。
//!
//! 扩展点（后续 ticket）：在 [`State`] 里加状态或字段，在 [`RegionSelection::handle`] 里处理
//! 新输入，在 [`RegionSelection::scene`] 里按层追加绘制元素。调整选区的几何计算在 `adjust`
//! 子模块；吸附（#11）可以在拖拽框选和 `adjust::dragged` 算出新边的那一步加入。

use bps_platform::{
    Color, Corner, Display, Label, OverlayCursor, OverlayInput, OverlayItem, OverlayKey,
    OverlayScene, Point, PointerButton, Rect,
};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 还没有选区，等待按下左键开始框选。
    Idle,
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
    /// 选区已锁定（开始标注），不再显示手柄和放大镜。
    locked: bool,
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
        })
    }

    /// 选区已锁定（例如开始标注）：此后不再显示手柄和放大镜。锁定后选区由别处接管输入，
    /// 这里不再收到输入。
    pub fn lock(&mut self) {
        self.locked = true;
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

    /// 已经框好、等待确定的选区；还没有选区或正在拖拽时为 `None`。
    pub fn selected(&self) -> Option<Rect> {
        match self.state {
            State::Selected(region) => Some(region),
            _ => None,
        }
    }

    /// 处理一次用户输入。
    pub fn handle(&mut self, input: &OverlayInput) -> Step {
        use OverlayInput::*;
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
                self.state =
                    State::Selected(adjust::dragged(original, grip, from, to, self.bounds));
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
                let anchor = self.clamp(position);
                self.state = State::Dragging {
                    anchor,
                    current: anchor,
                    previous: self.region(),
                };
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
                self.state = if current == anchor {
                    previous.map_or(State::Idle, State::Selected)
                } else {
                    State::Selected(between(anchor, current))
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
                State::Dragging { .. } | State::Selected(_) | State::Adjusting { .. },
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
            State::Dragging {
                anchor, current, ..
            } => Some(between(anchor, current)),
            State::Selected(region) => Some(region),
            State::Adjusting {
                original,
                grip,
                from,
                to,
            } => Some(adjust::dragged(original, grip, from, to, self.bounds)),
        }
    }

    /// 放大镜对准的像素；不显示放大镜时为 `None`。还没有选区时对准指针，拖拽框选时对准
    /// 拖动的角，拖动手柄时对准被拖动的边；选区框好后、移动选区时和锁定后不显示。
    pub fn magnifier_focus(&self) -> Option<Point> {
        if self.locked {
            return None;
        }
        match self.state {
            State::Idle => self.pointer,
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
            _ if self.locked => None,
            State::Adjusting { grip, .. } => Some(grip),
            State::Selected(region) => self.pointer.and_then(|p| self.grip_at(region, p)),
            State::Idle | State::Dragging { .. } => None,
        };
        grip.map_or(OverlayCursor::Crosshair, Grip::cursor)
    }

    /// 当前状态下遮罩上要画的内容，从下往上：遮挡、选区边框、标签。
    pub fn scene(&self) -> OverlayScene {
        let mut items = Vec::new();
        match self.region() {
            None => items.push(OverlayItem::Fill {
                rect: self.bounds,
                color: MASK,
            }),
            Some(region) => {
                items.extend(mask_around(self.bounds, region));
                let scale = self.scale_at(Point::new(region.x, region.y));
                let border = (scale.round() as u32).max(1);
                items.push(OverlayItem::Frame {
                    rect: region,
                    width: border,
                    color: ACCENT,
                });
                if !self.locked
                    && matches!(self.state, State::Selected(_) | State::Adjusting { .. })
                {
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
