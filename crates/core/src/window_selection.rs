//! 选区遮罩上选择窗口的交互状态机，以及每个状态下遮罩要画的场景。
//!
//! 平台无关：输入是平台递交的 [`OverlayInput`]，命中测试用拍冻结帧时取得的
//! [`WindowLayout`]，输出是下一步（继续 / 选中某窗口 / 取消）和描述遮罩画面的 [`OverlayScene`]。
//! 坐标一律是虚拟桌面物理像素。
//!
//! 交互规则：
//! - 指针下的顶层窗口（只看顶层窗口，不看里面的控件）高亮：它以外的部分变暗，内侧描边，
//!   左上角标出窗口截图的尺寸。遮罩一出现就按当时的光标位置高亮。
//! - 在高亮的窗口上单击左键（按下和松开都在同一个窗口上），或按 Enter，选中这个窗口。
//! - Esc、右键随时取消整次捕获；在没有窗口的地方单击什么也不做。

use bps_platform::{
    Corner, Display, Label, OverlayCursor, OverlayInput, OverlayItem, OverlayKey, OverlayScene,
    Point, PointerButton, Rect, TopLevelWindow, WindowId, WindowLayout,
};

use crate::selection::{
    ACCENT, FONT_SIZE, LABEL_BACKGROUND, LABEL_GAP, LABEL_PADDING, LABEL_TEXT, MASK, mask_around,
};
use crate::stitch::virtual_desktop_bounds;

/// 选择窗口的下一步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowStep {
    /// 还在选择。
    Continue,
    /// 用户选中了这个窗口。
    Confirmed(TopLevelWindow),
    /// 用户取消了这次捕获。
    Cancelled,
}

/// 一次窗口选择的交互状态。
pub(crate) struct WindowSelection {
    displays: Vec<Display>,
    /// 虚拟桌面范围。
    bounds: Rect,
    layout: WindowLayout,
    /// 指针下的窗口（高亮的窗口）。
    hovered: Option<WindowId>,
    /// 按下左键时指针下的窗口；松开时还在同一个窗口上才算单击选中。
    pressed: Option<WindowId>,
}

impl WindowSelection {
    /// 在冻结帧覆盖的这些显示器上、从 `layout` 里选择窗口，先高亮 `cursor` 处的窗口；
    /// 没有显示器时返回 `None`。
    pub fn new(
        displays: Vec<Display>,
        layout: WindowLayout,
        cursor: Option<Point>,
    ) -> Option<Self> {
        let bounds = virtual_desktop_bounds(&displays)?;
        let mut selection = Self {
            displays,
            bounds,
            layout,
            hovered: None,
            pressed: None,
        };
        selection.hovered = cursor.and_then(|point| selection.window_at(point));
        Some(selection)
    }

    /// 处理一次用户输入。
    pub fn handle(&mut self, input: &OverlayInput) -> WindowStep {
        match *input {
            OverlayInput::Interrupted
            | OverlayInput::KeyPressed {
                key: OverlayKey::Escape,
                ..
            }
            | OverlayInput::PointerReleased {
                button: PointerButton::Right,
                ..
            } => return WindowStep::Cancelled,
            OverlayInput::PointerMoved { position, .. } => {
                self.hovered = self.window_at(position);
            }
            OverlayInput::PointerPressed {
                button: PointerButton::Left,
                position,
                ..
            } => {
                self.hovered = self.window_at(position);
                self.pressed = self.hovered;
            }
            OverlayInput::PointerReleased {
                button: PointerButton::Left,
                position,
                ..
            } => {
                self.hovered = self.window_at(position);
                let clicked = self.pressed.take().filter(|id| self.hovered == Some(*id));
                if let Some(window) = clicked.and_then(|id| self.layout.get(id)) {
                    return WindowStep::Confirmed(window.clone());
                }
            }
            OverlayInput::KeyPressed {
                key: OverlayKey::Enter,
                ..
            } => {
                if let Some(window) = self.hovered.and_then(|id| self.layout.get(id)) {
                    return WindowStep::Confirmed(window.clone());
                }
            }
            // 窗口模式不接受文字输入。
            OverlayInput::PointerPressed {
                button: PointerButton::Right,
                ..
            }
            | OverlayInput::KeyPressed { .. }
            | OverlayInput::Character(_) => {}
        }
        WindowStep::Continue
    }

    /// 当前状态下遮罩上要画的内容，从下往上：遮挡、高亮窗口的内侧描边、尺寸标签。
    pub fn scene(&self) -> OverlayScene {
        let highlighted = self
            .hovered
            .and_then(|id| self.layout.get(id))
            .and_then(|window| {
                let visible = window.bounds.intersection(self.bounds)?;
                Some((window, visible))
            });
        let mut items = Vec::new();
        match highlighted {
            None => items.push(OverlayItem::Fill {
                rect: self.bounds,
                color: MASK,
            }),
            Some((window, visible)) => {
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
            }
        }
        OverlayScene {
            items,
            cursor: OverlayCursor::Arrow,
            text_caret: None,
        }
    }

    /// 命中测试：`point` 处用户看到的顶层窗口。
    fn window_at(&self, point: Point) -> Option<WindowId> {
        self.layout.window_at(point).map(|window| window.id)
    }

    /// `point` 所在显示器的缩放比例；不在任何显示器上时取主显示器的。
    fn scale_at(&self, point: Point) -> f64 {
        self.displays
            .iter()
            .find(|d| d.bounds.contains(point))
            .or_else(|| self.displays.iter().find(|d| d.is_primary))
            .or_else(|| self.displays.first())
            .map_or(1.0, |d| d.scale_factor)
    }
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
