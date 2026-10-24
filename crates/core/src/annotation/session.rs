//! 标注时的交互：工具条、在区域里画标注、输入文字、选择结束方式。
//!
//! [`Annotator`] 叠在选区状态机（`selection.rs`）之上，由区域捕获（`region.rs`）先把每个输入
//! 交给它，它不处理的（[`Response::Pass`]）再交给选区状态机：
//! - 选区刚框好（选区状态机处于“已选定”）时就显示工具条；这时用户仍可重新框选、按 Enter
//!   或双击直接复制，也可以先选好颜色和线宽。
//! - 用户点了某个工具，区域就锁定下来（[`Annotator::lock`]）：从冻结帧取出区域的截图作为
//!   底图，遮罩的画布开始显示合成后的截图，此后所有输入都由标注处理。
//! - 框、箭头、马赛克靠拖拽画出；文字工具在单击处开始输入（打开输入法），Enter 换行、
//!   退格删字，Esc、单击别处、换工具或结束捕获时这段文字定下来。改颜色和线宽只影响之后画的
//!   标注，以及正在输入的这段文字。
//! - 点“复制”“自动存储”“另存为”结束（[`Response::Finish`]）；点“取消”或按 Esc 取消。
//!
//! 标注的坐标是截图的像素坐标：屏幕坐标减去区域左上角。比屏幕还高的长截屏缩小显示在区域里
//! （[`Annotator::lock_image`]），屏幕坐标再按缩小比例换算；屏幕上显示的是把标注按同样比例
//! 缩小后画在缩小的截图上，最终截图仍按原尺寸合成。

use std::sync::Arc;

use bps_platform::{
    Color, Display, Image, OverlayCursor, OverlayInput, OverlayItem, OverlayKey, OverlayScene,
    Platform, PlatformError, Point, PointerButton, Rect, SelectionOverlay, TextMask,
};

use super::style::{PALETTE, Thickness};
use super::toolbar::{Button, Tool, Toolbar, ToolbarState};
use super::{Annotation, AnnotationDocument, Stroke, compose_area, offset};
use crate::Texts;
use crate::downscale::downscale;
use crate::region::frozen_region;
use crate::selection::between;

/// 输入文字时文字框的边框：内白外黑两层，深浅背景上都看得见。
const TEXT_BOX_INNER: Color = Color::rgba(255, 255, 255, 200);
const TEXT_BOX_OUTER: Color = Color::rgba(0, 0, 0, 140);
/// 文字框离文字的距离（100% 缩放下的物理像素）。
const TEXT_BOX_PADDING: f64 = 3.0;

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
    /// 之后画的标注用的颜色和线宽。
    color: Color,
    thickness: Thickness,
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
    /// 截图显示在屏幕上的范围。
    region: Rect,
    /// 截图（标注的底图），通常从冻结帧取出。
    base: Image,
    /// 截图缩小显示时屏幕上的样子；按原尺寸显示时为 `None`。
    view: Option<View>,
    document: AnnotationDocument,
    /// 正在拖拽画出的标注（截图坐标）。
    draft: Option<Draft>,
    /// 正在输入的文字。
    typing: Option<Typing>,
    /// 遮罩画布上正显示着的标注（屏幕上的尺寸）；画布还没显示时为 `None`。
    shown: Option<Vec<Annotation>>,
}

/// 缩小显示的截图：屏幕像素 / 截图像素的比例，以及缩小后的底图。
struct View {
    scale_x: f64,
    scale_y: f64,
    base: Image,
}

/// 正在拖拽画出的框、箭头或马赛克。
#[derive(Clone, Copy)]
struct Draft {
    tool: Tool,
    start: Point,
    end: Point,
    color: Color,
    thickness: Thickness,
    /// 开始画的那块显示器的缩放比例。
    scale: f64,
}

impl Draft {
    /// 拖出来的标注；没有拖动（起点等于终点）时为 `None`。
    fn annotation(&self) -> Option<Annotation> {
        if self.start == self.end {
            return None;
        }
        let stroke = Stroke {
            color: self.color,
            width: self.thickness.stroke_width_at(self.scale),
        };
        match self.tool {
            Tool::Rectangle => Some(Annotation::Rectangle {
                rect: between(self.start, self.end),
                stroke,
            }),
            Tool::Arrow => Some(Annotation::Arrow {
                from: self.start,
                to: self.end,
                stroke,
            }),
            Tool::Mosaic => Some(Annotation::Mosaic {
                rect: between(self.start, self.end),
                block: self.thickness.mosaic_block_at(self.scale),
            }),
            Tool::Text => None,
        }
    }
}

/// 正在输入的一段文字（截图坐标）。
struct Typing {
    /// 排版原点：单击的位置。
    origin: Point,
    text: String,
    color: Color,
    thickness: Thickness,
    /// 单击处那块显示器的缩放比例。
    scale: f64,
    /// `text` 当前的蒙版；没有文字时也有一行高的排版框和插入点。
    mask: Arc<TextMask>,
}

impl Typing {
    fn size(&self) -> u32 {
        self.thickness.font_size_at(self.scale)
    }

    /// 按当前的文字和字号重新栅格化。
    fn rasterize(&mut self, overlay: &dyn SelectionOverlay) -> Result<(), PlatformError> {
        self.mask = Arc::new(overlay.rasterize_text(&self.text, self.size())?);
        Ok(())
    }

    /// 这段文字的标注；还没有文字时为 `None`。
    fn annotation(&self) -> Option<Annotation> {
        (!self.text.is_empty()).then(|| Annotation::Text {
            origin: self.origin,
            text: self.text.clone(),
            size: self.size(),
            color: self.color,
            mask: self.mask.clone(),
        })
    }

    /// 文字框：排版框和插入点（截图坐标）。
    fn frame(&self) -> Rect {
        offset(self.mask.layout().union(self.mask.caret()), self.origin)
    }

    /// 插入点（截图坐标）。
    fn caret(&self) -> Rect {
        offset(self.mask.caret(), self.origin)
    }
}

impl Canvas {
    /// 现在应显示的标注：已画好的加上正在画的和正在输入的。
    fn annotations(&self) -> Vec<Annotation> {
        let mut annotations = self.document.annotations().to_vec();
        annotations.extend(self.draft.and_then(|d| d.annotation()));
        annotations.extend(self.typing.as_ref().and_then(Typing::annotation));
        annotations
    }

    /// 把正在拖拽画出的标注定下来。
    fn commit_draft(&mut self) {
        if let Some(annotation) = self.draft.take().and_then(|d| d.annotation()) {
            self.document.add(annotation);
        }
    }

    /// 结束输入：有文字时定下来，没有文字就不留下任何东西。
    fn commit_typing(&mut self) {
        if let Some(annotation) = self.typing.take().and_then(|t| t.annotation()) {
            self.document.add(annotation);
        }
    }

    /// 屏幕坐标 → 截图坐标，限制在截图以内。
    fn to_image(&self, point: Point) -> Point {
        let right = self.region.x + self.region.width as i32 - 1;
        let bottom = self.region.y + self.region.height as i32 - 1;
        let local = Point::new(
            point.x.clamp(self.region.x, right) - self.region.x,
            point.y.clamp(self.region.y, bottom) - self.region.y,
        );
        let Some(view) = &self.view else {
            return local;
        };
        let unscale = |v: i32, scale: f64, size: u32| {
            (((f64::from(v) + 0.5) / scale) as i32).clamp(0, size as i32 - 1)
        };
        Point::new(
            unscale(local.x, view.scale_x, self.base.width()),
            unscale(local.y, view.scale_y, self.base.height()),
        )
    }

    /// 截图坐标 → 屏幕坐标（截图缩小显示时按比例缩小）。
    fn to_screen(&self, rect: Rect) -> Rect {
        let rect = match &self.view {
            Some(view) => {
                let x0 = (f64::from(rect.x) * view.scale_x).round() as i32;
                let y0 = (f64::from(rect.y) * view.scale_y).round() as i32;
                let x1 = (f64::from(rect.x + rect.width as i32) * view.scale_x).round() as i32;
                let y1 = (f64::from(rect.y + rect.height as i32) * view.scale_y).round() as i32;
                Rect::new(x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32)
            }
            None => rect,
        };
        offset(rect, Point::new(self.region.x, self.region.y))
    }

    /// 显示器的缩放比例 `scale`（屏幕上的线宽、字号按它定）→ 截图上用的比例：截图缩小显示时
    /// 相应放大，屏幕上看起来仍是原来的粗细。
    fn to_image_scale(&self, scale: f64) -> f64 {
        match &self.view {
            Some(view) => scale / ((view.scale_x + view.scale_y) / 2.0).max(f64::EPSILON),
            None => scale,
        }
    }

    /// 屏幕上要显示的标注：按截图显示的比例缩放。
    fn displayed(&self, annotations: Vec<Annotation>) -> Vec<Annotation> {
        match &self.view {
            Some(view) => annotations
                .iter()
                .map(|a| a.scaled(view.scale_x, view.scale_y))
                .collect(),
            None => annotations,
        }
    }

    /// 屏幕上显示的底图。
    fn shown_base(&self) -> &Image {
        self.view.as_ref().map_or(&self.base, |view| &view.base)
    }
}

impl Annotator {
    /// 在这些显示器上标注，工具条文字用 `texts` 的语言。
    pub fn new(displays: Vec<Display>, texts: &'static Texts) -> Self {
        Self {
            displays,
            texts,
            tool: None,
            color: PALETTE[0],
            thickness: Thickness::default(),
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

    /// 正在输入的文字。
    fn typing(&self) -> Option<&Typing> {
        self.canvas.as_ref().and_then(|c| c.typing.as_ref())
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
        let typing = self.typing().is_some();
        let toolbar = self.toolbar(region);
        let response = match *input {
            OverlayInput::Interrupted if locked => Response::Cancel,
            OverlayInput::KeyPressed { key, .. } if typing => {
                match key {
                    OverlayKey::Escape => self.finish_typing(),
                    OverlayKey::Enter => self.edit_text(platform, |text| text.push('\n'))?,
                    OverlayKey::Backspace => self.edit_text(platform, |text| {
                        text.pop();
                    })?,
                    // 方向键、C 等在打字时没有含义。
                    _ => {}
                }
                Response::Handled
            }
            OverlayInput::Character(character) if typing => {
                self.edit_text(platform, |text| text.push(character))?;
                Response::Handled
            }
            OverlayInput::KeyPressed {
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
                    self.press_in_canvas(platform, position, region)?;
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
            // 用户看到的正在画的标注和正在输入的文字也算在最终截图里。
            canvas.commit_draft();
            canvas.commit_typing();
        }
        Ok(response)
    }

    /// 锁定后在工具条以外按下左键：点在正在输入的文字框里时继续输入；否则先结束输入，
    /// 再按当前工具在区域里开始输入新的文字或拖拽画标注。
    fn press_in_canvas(
        &mut self,
        platform: &Platform,
        position: Point,
        region: Rect,
    ) -> Result<(), PlatformError> {
        let scale = self.scale_at(position);
        let (tool, color, thickness) = (self.tool, self.color, self.thickness);
        let Some(canvas) = self.canvas.as_mut() else {
            return Ok(());
        };
        let point = canvas.to_image(position);
        let scale = canvas.to_image_scale(scale);
        if canvas
            .typing
            .as_ref()
            .is_some_and(|t| t.frame().contains(point))
        {
            return Ok(());
        }
        canvas.commit_typing();
        let Some(tool) = tool.filter(|_| region.contains(position)) else {
            return Ok(());
        };
        if tool == Tool::Text {
            let mut typing = Typing {
                origin: point,
                text: String::new(),
                color,
                thickness,
                scale,
                mask: Arc::new(TextMask::new(
                    Rect::default(),
                    Vec::new(),
                    Rect::default(),
                    Rect::default(),
                )),
            };
            typing.rasterize(platform.overlay.as_ref())?;
            canvas.typing = Some(typing);
        } else {
            canvas.draft = Some(Draft {
                tool,
                start: point,
                end: point,
                color,
                thickness,
                scale,
            });
        }
        Ok(())
    }

    /// 修改正在输入的文字并重新栅格化。
    fn edit_text(
        &mut self,
        platform: &Platform,
        edit: impl FnOnce(&mut String),
    ) -> Result<(), PlatformError> {
        if let Some(typing) = self.canvas.as_mut().and_then(|c| c.typing.as_mut()) {
            edit(&mut typing.text);
            typing.rasterize(platform.overlay.as_ref())?;
        }
        Ok(())
    }

    /// 结束输入，文字定下来。
    fn finish_typing(&mut self) {
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.commit_typing();
        }
    }

    /// 点击了工具条上的按钮。
    fn click(
        &mut self,
        platform: &Platform,
        button: Button,
        region: Rect,
    ) -> Result<Response, PlatformError> {
        let typing = self.canvas.as_mut().and_then(|c| c.typing.as_mut());
        Ok(match button {
            Button::Tool(tool) => {
                self.finish_typing();
                self.lock(platform, region)?;
                self.tool = Some(tool);
                Response::Handled
            }
            Button::Color(color) => {
                self.color = color;
                if let Some(typing) = typing {
                    typing.color = color;
                }
                Response::Handled
            }
            Button::Thickness(thickness) => {
                self.thickness = thickness;
                if let Some(typing) = typing {
                    typing.thickness = thickness;
                    typing.rasterize(platform.overlay.as_ref())?;
                }
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
            let base = frozen_region(platform, &self.displays, region)?;
            self.lock_image(region, base);
        }
        Ok(())
    }

    /// 在 `region` 里标注 `image`（例如拼接好的长截屏）：尺寸不同时把截图缩小显示在
    /// `region` 里。已经锁定时什么也不做。
    pub fn lock_image(&mut self, region: Rect, image: Image) {
        if self.canvas.is_some() {
            return;
        }
        let view =
            ((region.width, region.height) != (image.width(), image.height())).then(|| View {
                scale_x: f64::from(region.width) / f64::from(image.width().max(1)),
                scale_y: f64::from(region.height) / f64::from(image.height().max(1)),
                base: downscale(&image, region.width, region.height),
            });
        self.canvas = Some(Canvas {
            region,
            base: image,
            view,
            document: AnnotationDocument::new(),
            draft: None,
            typing: None,
            shown: None,
        });
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

    /// 让遮罩的画布显示当前的标注：第一次整块显示，之后只更新变化了的部分。
    /// 返回画布是否有变化（有变化时需要重画遮罩）。
    pub fn sync_canvas(
        &mut self,
        overlay: &mut dyn SelectionOverlay,
    ) -> Result<bool, PlatformError> {
        let Some(canvas) = self.canvas.as_mut() else {
            return Ok(false);
        };
        let current = canvas.displayed(canvas.annotations());
        let base = canvas.shown_base();
        let Some(shown) = canvas.shown.as_ref() else {
            let image = compose_area(&current, base, full(base));
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
            let patch = compose_area(&current, base, area);
            if patch.width() == 0 || patch.height() == 0 {
                continue;
            }
            let Some(area) = area.intersection(full(base)) else {
                continue;
            };
            let at = Point::new(canvas.region.x + area.x, canvas.region.y + area.y);
            overlay.update_canvas(at, &patch)?;
        }
        canvas.shown = Some(current);
        Ok(true)
    }

    /// 在选区状态机画好的场景上加上工具条和正在输入的文字框、插入点，并按鼠标位置设置
    /// 指针形状；正在输入时请平台打开输入法。
    pub fn decorate(&self, scene: &mut OverlayScene, selected: Option<Rect>) {
        let Some(region) = self.region(selected) else {
            return;
        };
        if let (Some(canvas), Some(typing)) = (self.canvas.as_ref(), self.typing()) {
            let scale = self.scale_at(Point::new(
                region.x + typing.origin.x,
                region.y + typing.origin.y,
            ));
            let padding = ((TEXT_BOX_PADDING * scale).round() as i32).max(1);
            let frame = canvas.to_screen(typing.frame());
            let frame = Rect::new(
                frame.x - padding,
                frame.y - padding,
                frame.width + 2 * padding as u32,
                frame.height + 2 * padding as u32,
            );
            let line = (scale.round() as u32).max(1);
            let caret = canvas.to_screen(typing.caret());
            scene.items.extend([
                OverlayItem::Frame {
                    rect: frame,
                    width: line,
                    color: TEXT_BOX_INNER,
                },
                OverlayItem::Frame {
                    rect: Rect::new(
                        frame.x - line as i32,
                        frame.y - line as i32,
                        frame.width + 2 * line,
                        frame.height + 2 * line,
                    ),
                    width: line,
                    color: TEXT_BOX_OUTER,
                },
                OverlayItem::Fill {
                    rect: caret,
                    color: typing.color,
                },
            ]);
            scene.text_caret = Some(caret);
        }
        let toolbar = self.toolbar(region);
        scene.items.extend(toolbar.items(&ToolbarState {
            tool: self.tool,
            color: self.color,
            thickness: self.thickness,
            hovered: self.hovered,
        }));
        let over_toolbar = self.pointer.is_some_and(|p| toolbar.contains(p));
        let drawing_here = self.canvas.is_some()
            && self.tool.is_some()
            && self.pointer.is_some_and(|p| region.contains(p));
        if over_toolbar || (self.canvas.is_some() && !drawing_here) {
            scene.cursor = OverlayCursor::Arrow;
        } else if drawing_here && self.tool == Some(Tool::Text) {
            scene.cursor = OverlayCursor::IBeam;
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
