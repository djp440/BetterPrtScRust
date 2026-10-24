//! 标注时的交互：工具条、在区域里拖拽画标注、选择结束方式。
//!
//! [`Annotator`] 叠在选区状态机（`selection.rs`）之上，由区域捕获（`region.rs`）先把每个输入
//! 交给它，它不处理的（[`Response::Pass`]）再交给选区状态机：
//! - 选区刚框好（选区状态机处于“已选定”）时就显示工具条；这时用户仍可重新框选、按 Enter
//!   或双击直接复制。
//! - 用户点了某个工具，区域就锁定下来（[`Annotator::lock`]）：从冻结帧取出区域的截图作为
//!   底图，遮罩的画布开始显示合成后的截图，此后所有输入都由标注处理。
//! - 点“复制”“自动存储”“另存为”结束（[`Response::Finish`]）；点“取消”或按 Esc 取消。
//!
//! 标注的坐标是截图的像素坐标：屏幕坐标减去区域左上角。

use bps_platform::{
    Color, Display, Image, OverlayCursor, OverlayInput, OverlayKey, OverlayScene, Platform,
    PlatformError, Point, PointerButton, Rect, SelectionOverlay,
};

use super::toolbar::{Button, Tool, Toolbar};
use super::{Annotation, AnnotationDocument, Stroke, compose_area};
use crate::Texts;
use crate::region::frozen_region;
use crate::selection::between;

/// 默认的标注颜色。
const DEFAULT_COLOR: Color = Color::rgba(240, 40, 40, 255);
/// 默认线宽（100% 缩放下的物理像素），按开始画的那块显示器的缩放比例放大。
const DEFAULT_WIDTH: f64 = 3.0;

/// 结束这次标注的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Finish {
    CopyToClipboard,
    AutoSave,
    SaveAs,
}

/// [`Annotator::handle`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Response {
    /// 与标注无关，交给选区状态机。
    Pass,
    /// 已处理，需要重画。
    Handled,
    /// 用户选择了结束方式。
    Finish(Finish),
    /// 用户取消了这次捕获。
    Cancel,
}

/// 一次捕获里的标注界面。
pub(crate) struct Annotator {
    displays: Vec<Display>,
    /// 工具条文字的语言。
    texts: &'static Texts,
    tool: Option<Tool>,
    /// 鼠标悬停的按钮。
    hovered: Option<Button>,
    /// 在哪个按钮上按下了左键（松开时还在同一个按钮上才算点击）。
    pressed: Option<Button>,
    pointer: Option<Point>,
    /// 区域锁定后的标注；锁定前为 `None`。
    canvas: Option<Canvas>,
}

/// 锁定的区域、它的截图和上面的标注。
struct Canvas {
    region: Rect,
    /// 区域的截图（标注的底图），从冻结帧取出。
    base: Image,
    document: AnnotationDocument,
    /// 正在拖拽画出的标注（截图坐标）。
    draft: Option<Draft>,
    /// 遮罩画布上正显示着的标注；画布还没显示时为 `None`。
    shown: Option<Vec<Annotation>>,
}

#[derive(Clone, Copy)]
struct Draft {
    tool: Tool,
    start: Point,
    end: Point,
    stroke: Stroke,
}

impl Draft {
    /// 拖出来的标注；没有拖动（起点等于终点）时为 `None`。
    fn annotation(&self) -> Option<Annotation> {
        if self.start == self.end {
            return None;
        }
        Some(match self.tool {
            Tool::Rectangle => Annotation::Rectangle {
                rect: between(self.start, self.end),
                stroke: self.stroke,
            },
            Tool::Arrow => Annotation::Arrow {
                from: self.start,
                to: self.end,
                stroke: self.stroke,
            },
        })
    }
}

impl Canvas {
    /// 现在应显示的标注：已画好的加上正在画的。
    fn annotations(&self) -> Vec<Annotation> {
        let mut annotations = self.document.annotations().to_vec();
        annotations.extend(self.draft.and_then(|d| d.annotation()));
        annotations
    }

    /// 把正在画的标注定下来。
    fn commit_draft(&mut self) {
        if let Some(annotation) = self.draft.take().and_then(|d| d.annotation()) {
            self.document.add(annotation);
        }
    }

    /// 屏幕坐标 → 截图坐标，限制在截图以内。
    fn to_image(&self, point: Point) -> Point {
        let right = self.region.x + self.region.width as i32 - 1;
        let bottom = self.region.y + self.region.height as i32 - 1;
        Point::new(
            point.x.clamp(self.region.x, right) - self.region.x,
            point.y.clamp(self.region.y, bottom) - self.region.y,
        )
    }
}

impl Annotator {
    /// 在这些显示器上标注，工具条文字用 `texts` 的语言。
    pub fn new(displays: Vec<Display>, texts: &'static Texts) -> Self {
        Self {
            displays,
            texts,
            tool: None,
            hovered: None,
            pressed: None,
            pointer: None,
            canvas: None,
        }
    }

    /// 标注所在的区域：锁定后是锁定的区域，否则是选区状态机里已选定的区域。
    fn region(&self, selected: Option<Rect>) -> Option<Rect> {
        self.canvas.as_ref().map(|c| c.region).or(selected)
    }

    /// 处理一次输入。`selected` 是选区状态机里已选定的区域（正在拖拽或还没有选区时为 `None`）。
    pub fn handle(
        &mut self,
        platform: &Platform,
        input: &OverlayInput,
        selected: Option<Rect>,
    ) -> Result<Response, PlatformError> {
        let Some(region) = self.region(selected) else {
            self.hovered = None;
            self.pressed = None;
            return Ok(Response::Pass);
        };
        let locked = self.canvas.is_some();
        let toolbar = self.toolbar(region);
        let response = match *input {
            OverlayInput::Interrupted
            | OverlayInput::KeyPressed {
                key: OverlayKey::Escape,
                ..
            } if locked => Response::Cancel,
            OverlayInput::KeyPressed {
                key: OverlayKey::Enter,
                ..
            } if locked => Response::Finish(Finish::CopyToClipboard),
            OverlayInput::PointerMoved { position, .. } => {
                self.pointer = Some(position);
                self.hovered = toolbar.button_at(position);
                if let Some(canvas) = self.canvas.as_mut()
                    && canvas.draft.is_some()
                {
                    let end = canvas.to_image(position);
                    if let Some(draft) = canvas.draft.as_mut() {
                        draft.end = end;
                    }
                }
                // 选区状态机也要看到鼠标移动（拖拽框选时）；锁定后它不再参与。
                if locked {
                    Response::Handled
                } else {
                    Response::Pass
                }
            }
            OverlayInput::PointerPressed {
                button: PointerButton::Left,
                position,
                double_click,
                ..
            } => {
                self.pointer = Some(position);
                if toolbar.contains(position) {
                    self.pressed = toolbar.button_at(position);
                    Response::Handled
                } else if !locked {
                    Response::Pass
                } else if double_click && region.contains(position) {
                    Response::Finish(Finish::CopyToClipboard)
                } else {
                    if let (Some(tool), true) = (self.tool, region.contains(position)) {
                        let stroke = self.stroke_at(position);
                        if let Some(canvas) = self.canvas.as_mut() {
                            let start = canvas.to_image(position);
                            canvas.draft = Some(Draft {
                                tool,
                                start,
                                end: start,
                                stroke,
                            });
                        }
                    }
                    Response::Handled
                }
            }
            OverlayInput::PointerReleased {
                button: PointerButton::Left,
                position,
                ..
            } => {
                self.pointer = Some(position);
                if let Some(pressed) = self.pressed.take() {
                    if toolbar.button_at(position) == Some(pressed) {
                        self.click(platform, pressed, region)?
                    } else {
                        Response::Handled
                    }
                } else if let Some(canvas) = self.canvas.as_mut() {
                    if canvas.draft.is_some() {
                        let end = canvas.to_image(position);
                        if let Some(draft) = canvas.draft.as_mut() {
                            draft.end = end;
                        }
                        canvas.commit_draft();
                    }
                    Response::Handled
                } else {
                    Response::Pass
                }
            }
            _ if locked => Response::Handled,
            _ => Response::Pass,
        };
        if let Response::Finish(_) = response
            && let Some(canvas) = self.canvas.as_mut()
        {
            // 用户看到的正在画的标注也算在最终截图里。
            canvas.commit_draft();
        }
        Ok(response)
    }

    /// 点击了工具条上的按钮。
    fn click(
        &mut self,
        platform: &Platform,
        button: Button,
        region: Rect,
    ) -> Result<Response, PlatformError> {
        Ok(match button {
            Button::Tool(tool) => {
                self.lock(platform, region)?;
                self.tool = Some(tool);
                Response::Handled
            }
            Button::Cancel => Response::Cancel,
            Button::SaveAs => Response::Finish(Finish::SaveAs),
            Button::AutoSave => Response::Finish(Finish::AutoSave),
            Button::Copy => Response::Finish(Finish::CopyToClipboard),
        })
    }

    /// 锁定区域：从冻结帧取出它的截图作为标注的底图。已经锁定时什么也不做。
    pub fn lock(&mut self, platform: &Platform, region: Rect) -> Result<(), PlatformError> {
        if self.canvas.is_none() {
            self.canvas = Some(Canvas {
                region,
                base: frozen_region(platform, &self.displays, region)?,
                document: AnnotationDocument::new(),
                draft: None,
                shown: None,
            });
        }
        Ok(())
    }

    /// 贴着 `region` 的工具条，让开选区边框（宽度与选区状态机画的一致：按区域左上角所在
    /// 显示器的缩放比例）。
    fn toolbar(&self, region: Rect) -> Toolbar {
        let frame = (self.scale_at(Point::new(region.x, region.y)).round() as u32).max(1);
        Toolbar::beside(region, frame, &self.displays, self.texts)
    }

    /// `point` 所在显示器的缩放比例；不在任何显示器上时为 1。
    fn scale_at(&self, point: Point) -> f64 {
        self.displays
            .iter()
            .find(|d| d.bounds.contains(point))
            .map_or(1.0, |d| d.scale_factor)
    }

    /// 在 `point` 开始画的标注的线条样式：线宽按所在显示器的缩放比例放大。
    fn stroke_at(&self, point: Point) -> Stroke {
        let scale = self.scale_at(point);
        Stroke {
            color: DEFAULT_COLOR,
            width: ((DEFAULT_WIDTH * scale).round() as u32).max(1),
        }
    }

    /// 让遮罩的画布显示当前的标注：第一次整块显示，之后只更新变化了的部分。
    /// 返回画布是否有变化（有变化时需要重画遮罩）。
    pub fn sync_canvas(
        &mut self,
        overlay: &mut dyn SelectionOverlay,
    ) -> Result<bool, PlatformError> {
        let Some(canvas) = self.canvas.as_mut() else {
            return Ok(false);
        };
        let current = canvas.annotations();
        let Some(shown) = canvas.shown.as_ref() else {
            let image = compose_area(&current, &canvas.base, full(&canvas.base));
            overlay.show_canvas(canvas.region, &image)?;
            canvas.shown = Some(current);
            return Ok(true);
        };
        // 从第一个不同的标注起，新旧标注画到的地方都要重新合成。
        let unchanged = shown
            .iter()
            .zip(&current)
            .take_while(|(a, b)| a == b)
            .count();
        if unchanged == shown.len() && unchanged == current.len() {
            return Ok(false);
        }
        let dirty: Vec<Rect> = shown[unchanged..]
            .iter()
            .chain(&current[unchanged..])
            .flat_map(Annotation::footprint)
            .collect();
        for area in dirty {
            let patch = compose_area(&current, &canvas.base, area);
            if patch.width() == 0 || patch.height() == 0 {
                continue;
            }
            let Some(area) = area.intersection(full(&canvas.base)) else {
                continue;
            };
            let at = Point::new(canvas.region.x + area.x, canvas.region.y + area.y);
            overlay.update_canvas(at, &patch)?;
        }
        canvas.shown = Some(current);
        Ok(true)
    }

    /// 在选区状态机画好的场景上加上工具条，并按鼠标位置设置指针形状。
    pub fn decorate(&self, scene: &mut OverlayScene, selected: Option<Rect>) {
        let Some(region) = self.region(selected) else {
            return;
        };
        let toolbar = self.toolbar(region);
        scene.items.extend(toolbar.items(self.tool, self.hovered));
        let over_toolbar = self.pointer.is_some_and(|p| toolbar.contains(p));
        let drawing_here = self.canvas.is_some()
            && self.tool.is_some()
            && self.pointer.is_some_and(|p| region.contains(p));
        if over_toolbar || (self.canvas.is_some() && !drawing_here) {
            scene.cursor = OverlayCursor::Arrow;
        } else if drawing_here {
            scene.cursor = OverlayCursor::Crosshair;
        }
    }

    /// 最终的截图：锁定后是合成了全部标注的截图，否则直接从冻结帧取出 `region`。
    pub fn final_image(&self, platform: &Platform, region: Rect) -> Result<Image, PlatformError> {
        match &self.canvas {
            Some(canvas) => Ok(canvas.document.compose(&canvas.base)),
            None => frozen_region(platform, &self.displays, region),
        }
    }

    /// 已锁定的区域。
    pub fn locked_region(&self) -> Option<Rect> {
        self.canvas.as_ref().map(|c| c.region)
    }
}

/// 整张截图的范围。
fn full(image: &Image) -> Rect {
    Rect::new(0, 0, image.width(), image.height())
}
