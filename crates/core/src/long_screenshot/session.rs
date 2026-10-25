//! 进行中的长截屏：在选区遮罩上框选区域 → 按“开始”后区域透过遮罩、用户自己滚动内容，
//! 平台持续采样区域的实时画面并逐帧拼接，区域旁边显示拼接预览 → 按“完成”（或高度到达上限
//! 自动停止并提示）→ 直达功能的长截屏直接复制到剪贴板（绑定开了自动保存时同时存盘）；
//! 从等待确认界面转入的长截屏回到等待确认：在遮罩上标注整张长截屏（比屏幕高时缩小显示），
//! 再确认、另存为或取消。
//!
//! 与区域捕获一样跨越多个平台事件：[`LongScreenshotCapture::start`] 打开遮罩后立即返回，
//! 之后每个 [`OverlayInput`] 交给 [`LongScreenshotCapture::handle`]，每个
//! `PlatformEvent::RegionSampled` 交给 [`LongScreenshotCapture::on_sample`]，直到完成或取消。
//!
//! 拼接的第一帧取自冻结帧（用户框选时看到的画面），之后的帧来自 `RegionSampler`。

use bps_platform::{
    Backdrop, Color, Corner, Display, IncomingRequest, Label, LocalDateTime, Notification,
    OverlayCursor, OverlayInput, OverlayItem, OverlayKey, OverlayScene, Platform, PlatformError,
    Point, PointerButton, Rect,
};

use super::{LongScreenshot, Stitched, preview};
use crate::annotation::finish::{self, Origin};
use crate::annotation::session::{Annotator, Response};
use crate::annotation::toolbar::{Entry, Toolbar, ToolbarButton};
use crate::capture::{CaptureFailure, Screenshot, display_at};
use crate::magnifier::Magnifier;
use crate::region::{Ending, frozen_region};
use crate::selection::{
    ACCENT, FONT_SIZE, LABEL_BACKGROUND, LABEL_GAP, LABEL_PADDING, LABEL_TEXT, RegionSelection,
    Step, mask_around,
};
use crate::stitch::virtual_desktop_bounds;
use crate::{CaptureMode, CaptureRequest, CoreError, Texts, action};

/// 拼接预览的边框。
const PREVIEW_BORDER: Color = Color::rgba(255, 255, 255, 160);

/// 长截屏控制条上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Start,
    Finish,
    Cancel,
}

impl ToolbarButton for Control {
    fn message(self) -> Option<&'static str> {
        Some(match self {
            Control::Start => "long-screenshot-start",
            Control::Finish => "long-screenshot-finish",
            Control::Cancel => "long-screenshot-cancel",
        })
    }
}

/// 框好区域、还没开始时的控制条。
const SELECTING_BAR: [Entry<Control>; 2] = [
    Entry::Button(Control::Cancel),
    Entry::Button(Control::Start),
];
/// 拼接期间的控制条。
const SAMPLING_BAR: [Entry<Control>; 2] = [
    Entry::Button(Control::Cancel),
    Entry::Button(Control::Finish),
];

/// 长截屏进行到哪一步。
enum Phase {
    /// 在冻结帧上框选区域。
    Selecting(RegionSelection),
    /// 区域透过遮罩，用户滚动内容，持续采样拼接。
    Sampling(Box<Sampling>),
    /// 在遮罩上标注拼好的长截屏；`view` 是它（可能缩小后）显示的范围。
    Annotating {
        annotator: Box<Annotator>,
        view: Rect,
    },
    /// 已经结束（只在交接时短暂出现）。
    Over,
}

/// 采样拼接中的状态。
struct Sampling {
    region: Rect,
    long: LongScreenshot,
    /// 预览框（预览画面贴着它的左上角，不超出它）。
    preview_box: Rect,
    /// 遮罩画布上正显示着的预览画面的范围；拼接结果变化后为 `None`，下次重画时更新。
    preview_shown: Option<Rect>,
    /// 最近一帧和已拼接的内容对不上（滚动太快）。
    lost: bool,
}

/// 进行中的长截屏。
pub(crate) struct LongScreenshotCapture {
    request: CaptureRequest,
    /// 拍下冻结帧的时间，即这次捕获发生的时间。
    captured_at: LocalDateTime,
    /// 冻结帧覆盖的显示器。
    displays: Vec<Display>,
    /// 虚拟桌面范围。
    bounds: Rect,
    phase: Phase,
    /// 鼠标悬停的、按下左键的控制条按钮（松开时还在同一个按钮上才算点击）。
    hovered: Option<Control>,
    pressed: Option<Control>,
    pointer: Option<Point>,
    /// 遮罩上当前画着的场景；场景没变时不重画。
    drawn: Option<OverlayScene>,
    /// 框选区域时的放大镜。
    magnifier: Magnifier,
    /// 开始捕获时的界面语言：遮罩上的文字和通知都用它。
    texts: &'static Texts,
    /// 从等待确认界面转入的：拼好后回到等待确认（标注、确认、另存为、取消），
    /// 而不是直接交付。
    confirm_after: bool,
    /// 经 CLI 转交来、等捕获结束再回复的请求。
    pub requester: Option<IncomingRequest>,
}

impl LongScreenshotCapture {
    /// 从等待确认界面转入：以已经选好的 `region` 直接开始采样（不再框选，也不用再按“开始”）。
    /// 这个会话打开自己的选区遮罩（拍新的冻结帧）。
    pub fn start_in(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
        region: Rect,
    ) -> Result<Self, CoreError> {
        let mut capture = Self::start(platform, request, texts)?;
        capture.confirm_after = true;
        match capture.begin_sampling(platform, region) {
            None => Ok(capture),
            Some(Err(failure)) => Err(failure.error),
            Some(Ok(_)) => unreachable!("开始采样不会直接结束"),
        }
    }

    /// 拍下冻结帧并显示选区遮罩。显示器、虚拟桌面模式不用框选：区域一开始就是整块显示器
    /// 或整个虚拟桌面，用户直接按“开始”。
    pub fn start(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
    ) -> Result<Self, CoreError> {
        let captured_at = platform.clock.now();
        let displays = platform.overlay.open(Backdrop::FrozenFrame)?;
        let (Some(selection), Some(bounds)) = (
            RegionSelection::new(displays.clone(), String::new()),
            virtual_desktop_bounds(&displays),
        ) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        let mut selection = selection.without_selected_hint();
        let preset = match request.mode {
            CaptureMode::Display => platform
                .frame_source
                .cursor_position()
                .ok()
                .and_then(|c| display_at(&displays, c))
                .map(|d| d.bounds),
            CaptureMode::VirtualDesktop => Some(bounds),
            CaptureMode::Region | CaptureMode::Window => None,
        };
        if let Some(region) = preset {
            selection = selection.with_selected(region);
        }
        let mut capture = Self {
            request,
            captured_at,
            magnifier: Magnifier::new(displays.clone(), texts),
            displays,
            bounds,
            phase: Phase::Selecting(selection),
            hovered: None,
            pressed: None,
            pointer: None,
            drawn: None,
            texts,
            confirm_after: false,
            requester: None,
        };
        if let Err(e) = capture.redraw(platform) {
            platform.overlay.close();
            return Err(e.into());
        }
        Ok(capture)
    }

    /// 处理用户在遮罩上的一次输入。返回 `None` 表示还在进行；否则这次捕获已经结束，
    /// 遮罩已关闭、采样已停止。
    pub fn handle(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match &self.phase {
            Phase::Selecting(_) => self.handle_selecting(platform, input),
            Phase::Sampling(_) => self.handle_sampling(platform, input),
            Phase::Annotating { .. } => self.handle_annotating(platform, input),
            Phase::Over => None,
        }
    }

    /// 平台采到了新画面：拼上去，更新预览；高度到达上限时自动完成。
    pub fn on_sample(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Sampling(sampling) = &mut self.phase else {
            // 已经不在采样（例如刚按了“完成”），这是之前排队的事件。
            return None;
        };
        let frame = match platform.region_sampler.take_sample() {
            Ok(Some(frame)) => frame,
            Ok(None) => return None,
            Err(e) => return Some(Err(self.fail(platform, e))),
        };
        match sampling.long.push(&frame) {
            Stitched::Full => return self.complete(platform),
            Stitched::Lost => sampling.lost = true,
            Stitched::Unchanged => sampling.lost = false,
            Stitched::Scrolled { .. } => {
                sampling.lost = false;
                sampling.preview_shown = None;
            }
        }
        self.keep_going(platform)
    }

    /// 放弃这次长截屏（例如退出程序）：停止采样、关闭遮罩。
    pub fn abandon(&mut self, platform: &mut Platform) {
        self.stop_sampling(platform);
        platform.overlay.close();
        self.phase = Phase::Over;
    }

    fn handle_selecting(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Selecting(selection) = &self.phase else {
            return None;
        };
        let selected = selection.selected();
        let bar = selected.map(|region| self.bar(&SELECTING_BAR, region));
        match self.control_input(bar.as_ref(), input) {
            ControlInput::Clicked(Control::Start) => {
                return self.begin_sampling(platform, selected?);
            }
            ControlInput::Clicked(_) => return self.cancel(platform),
            ControlInput::Consumed => return self.keep_going(platform),
            ControlInput::Pass => {}
        }
        let Phase::Selecting(selection) = &mut self.phase else {
            return None;
        };
        match selection.handle(input) {
            Step::Continue => self.keep_going(platform),
            // Enter 或双击选区等于点“开始”。
            Step::Confirmed(region) => self.begin_sampling(platform, region),
            Step::Cancelled => self.cancel(platform),
            Step::CopyColor(focus) => match self.magnifier.copy_color(platform, focus) {
                Ok(()) => self.keep_going(platform),
                Err(e) => Some(Err(self.fail(platform, e))),
            },
        }
    }

    fn handle_sampling(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Sampling(sampling) = &self.phase else {
            return None;
        };
        let region = sampling.region;
        let bar = self.bar(&SAMPLING_BAR, region);
        match self.control_input(Some(&bar), input) {
            ControlInput::Clicked(Control::Finish) => return self.complete(platform),
            ControlInput::Clicked(_) => return self.cancel(platform),
            ControlInput::Consumed | ControlInput::Pass => {}
        }
        match *input {
            OverlayInput::Interrupted
            | OverlayInput::KeyPressed {
                key: OverlayKey::Escape,
                ..
            } => self.cancel(platform),
            OverlayInput::KeyPressed {
                key: OverlayKey::Enter,
                ..
            } => self.complete(platform),
            _ => self.keep_going(platform),
        }
    }

    fn handle_annotating(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Annotating { annotator, view } = &mut self.phase else {
            return None;
        };
        let view = *view;
        match annotator.handle(platform, input, Some(view)) {
            // 这里的工具条没有转入其他功能的按钮。
            Ok(Response::Pass | Response::Handled | Response::Switch(_)) => {
                self.keep_going(platform)
            }
            Ok(Response::Finish(how)) => {
                let Phase::Annotating { annotator, .. } = &self.phase else {
                    return None;
                };
                match finish::conclude(
                    platform,
                    annotator,
                    view,
                    &self.request,
                    &Origin {
                        captured_at: self.captured_at,
                        window_title: None,
                    },
                    how,
                    self.texts,
                ) {
                    None => self.keep_going(platform),
                    Some(result) => {
                        self.phase = Phase::Over;
                        Some(result.map(|()| Ending::Delivered))
                    }
                }
            }
            Ok(Response::Cancel) => self.cancel(platform),
            Err(e) => Some(Err(self.fail(platform, e))),
        }
    }

    /// 控制条对这次输入的反应。
    fn control_input(
        &mut self,
        bar: Option<&Toolbar<Control>>,
        input: &OverlayInput,
    ) -> ControlInput {
        let at = |point: Point| bar.and_then(|bar| bar.button_at(point));
        match *input {
            OverlayInput::PointerMoved { position, .. } => {
                self.pointer = Some(position);
                self.hovered = at(position);
                ControlInput::Pass
            }
            OverlayInput::PointerPressed {
                button: PointerButton::Left,
                position,
                ..
            } => {
                self.pointer = Some(position);
                if bar.is_some_and(|bar| bar.contains(position)) {
                    self.pressed = at(position);
                    ControlInput::Consumed
                } else {
                    ControlInput::Pass
                }
            }
            OverlayInput::PointerReleased {
                button: PointerButton::Left,
                position,
                ..
            } => {
                self.pointer = Some(position);
                match self.pressed.take() {
                    Some(pressed) if at(position) == Some(pressed) => {
                        ControlInput::Clicked(pressed)
                    }
                    Some(_) => ControlInput::Consumed,
                    None => ControlInput::Pass,
                }
            }
            _ => ControlInput::Pass,
        }
    }

    /// 贴着 `region` 的控制条。
    fn bar(&self, entries: &[Entry<Control>], region: Rect) -> Toolbar<Control> {
        Toolbar::with_entries(
            entries,
            region,
            self.border(region),
            &self.displays,
            self.texts,
        )
    }

    /// 区域边框的宽度：区域所在显示器的缩放比例对应的像素数。
    fn border(&self, region: Rect) -> u32 {
        let scale = preview::display_of(region, &self.displays).map_or(1.0, |d| d.scale_factor);
        (scale.round() as u32).max(1)
    }

    /// 用户按了“开始”：冻结帧里的区域作为第一帧，区域透过遮罩，开始采样。
    fn begin_sampling(
        &mut self,
        platform: &mut Platform,
        region: Rect,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let border = self.border(region);
        let (preview_box, _) = preview::preview_box(region, border, &self.displays);
        let bar = self.bar(&SAMPLING_BAR, region);
        let started = frozen_region(platform, &self.displays, region).and_then(|first| {
            // 控制条、预览放不到区域外面时盖在区域里，这部分不透过遮罩。
            let open = clear_of(region, &[bar.bounds(), preview_box]);
            platform.overlay.set_pass_through(Some(open))?;
            platform.region_sampler.start(region)?;
            Ok(first)
        });
        let first = match started {
            Ok(first) => first,
            Err(e) => return Some(Err(self.fail(platform, e))),
        };
        self.phase = Phase::Sampling(Box::new(Sampling {
            region,
            long: LongScreenshot::new(first),
            preview_box,
            preview_shown: None,
            lost: false,
        }));
        self.hovered = None;
        self.pressed = None;
        self.keep_going(platform)
    }

    /// 结束拼接：停止采样；到达高度上限时提示用户。从等待确认界面转入的回到等待确认（进入标注），
    /// 否则关闭遮罩，复制到剪贴板（绑定开了自动保存时同时存盘）。
    fn complete(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        self.stop_sampling(platform);
        let Phase::Sampling(sampling) = std::mem::replace(&mut self.phase, Phase::Over) else {
            return None;
        };
        let Sampling { region, long, .. } = *sampling;
        if long.is_full() {
            platform.notifier.notify(Notification {
                title: self.texts.get("notification-long-screenshot-full-title"),
                body: self.texts.format(
                    "notification-long-screenshot-full-body",
                    &[("limit", &LongScreenshot::MAX_HEIGHT)],
                ),
                on_click: None,
            });
        }
        let image = long.into_image();
        if self.confirm_after {
            let view = preview::annotation_view(region, &image, &self.displays);
            let mut annotator = Annotator::new(self.displays.clone(), self.texts);
            annotator.lock_image(view, image);
            self.phase = Phase::Annotating {
                annotator: Box::new(annotator),
                view,
            };
            self.hovered = None;
            self.pressed = None;
            return self.keep_going(platform);
        }
        platform.overlay.close();
        let screenshot = Screenshot {
            image,
            mode: self.request.mode,
            captured_at: self.captured_at,
            window_title: None,
        };
        Some(
            action::run(platform, &screenshot, &self.request, self.texts)
                .map(|()| Ending::Delivered),
        )
    }

    /// 用户取消：停止采样、关闭遮罩，没有任何产物。
    fn cancel(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        self.abandon(platform);
        Some(Ok(Ending::Cancelled))
    }

    /// 出错了：停止采样、关闭遮罩，捕获失败。
    fn fail(&mut self, platform: &mut Platform, error: PlatformError) -> CaptureFailure {
        self.abandon(platform);
        CaptureFailure::before_actions(error)
    }

    fn stop_sampling(&mut self, platform: &mut Platform) {
        if matches!(self.phase, Phase::Sampling(_)) {
            platform.region_sampler.stop();
            // 遮罩接着用（标注）或马上关闭，恢复失败也不影响结果。
            let _ = platform.overlay.set_pass_through(None);
        }
    }

    /// 捕获还在进行：重画遮罩。重画失败时停止采样、关闭遮罩，捕获失败。
    fn keep_going(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        match self.redraw(platform) {
            Ok(()) => None,
            Err(e) => Some(Err(self.fail(platform, e))),
        }
    }

    fn redraw(&mut self, platform: &mut Platform) -> Result<(), PlatformError> {
        let canvas_changed = match &mut self.phase {
            Phase::Sampling(sampling) if sampling.preview_shown.is_none() => {
                let (area, image) = preview::render(&sampling.long, sampling.preview_box);
                platform.overlay.show_canvas(area, &image)?;
                sampling.preview_shown = Some(area);
                true
            }
            Phase::Annotating { annotator, .. } => {
                annotator.sync_canvas(platform.overlay.as_mut())?
            }
            Phase::Over => return Ok(()),
            Phase::Selecting(_) | Phase::Sampling(_) => false,
        };
        let mut scene = self.scene();
        if let Phase::Selecting(selection) = &self.phase
            && let Some(focus) = selection.magnifier_focus()
        {
            let magnifier = self.magnifier.items(platform.overlay.as_ref(), focus)?;
            scene.items.extend(magnifier);
        }
        if canvas_changed || self.drawn.as_ref() != Some(&scene) {
            platform.overlay.render(&scene)?;
            self.drawn = Some(scene);
        }
        Ok(())
    }

    /// 遮罩上现在要画的内容。
    fn scene(&self) -> OverlayScene {
        match &self.phase {
            Phase::Selecting(selection) => {
                let mut scene = selection.scene();
                if let Some(region) = selection.selected() {
                    let bar = self.bar(&SELECTING_BAR, region);
                    scene
                        .items
                        .extend(bar.items_with(|_| false, None, self.hovered));
                    if self.pointer.is_some_and(|p| bar.contains(p)) {
                        scene.cursor = OverlayCursor::Arrow;
                    }
                }
                scene
            }
            Phase::Sampling(sampling) => self.sampling_scene(sampling),
            Phase::Annotating { annotator, view } => {
                let mut items = mask_around(self.bounds, *view);
                items.push(OverlayItem::Frame {
                    rect: *view,
                    width: self.border(*view),
                    color: ACCENT,
                });
                let mut scene = OverlayScene {
                    items,
                    cursor: OverlayCursor::Arrow,
                    text_caret: None,
                };
                annotator.decorate(&mut scene, Some(*view));
                scene
            }
            Phase::Over => OverlayScene {
                items: Vec::new(),
                cursor: OverlayCursor::Arrow,
                text_caret: None,
            },
        }
    }

    /// 采样期间的遮罩：区域和预览以外的遮挡、区域边框、预览边框和下方的状态标签、控制条。
    fn sampling_scene(&self, sampling: &Sampling) -> OverlayScene {
        let border = self.border(sampling.region);
        let preview = sampling.preview_shown.unwrap_or(sampling.preview_box);
        let mut items: Vec<OverlayItem> = mask_around(self.bounds, sampling.region)
            .into_iter()
            .flat_map(|item| match item {
                OverlayItem::Fill { rect, color } => subtract(rect, preview)
                    .into_iter()
                    .map(|rect| OverlayItem::Fill { rect, color })
                    .collect(),
                other => vec![other],
            })
            .collect();
        items.push(OverlayItem::Frame {
            rect: sampling.region,
            width: border,
            color: ACCENT,
        });
        items.push(OverlayItem::Frame {
            rect: preview,
            width: border,
            color: PREVIEW_BORDER,
        });
        let text = if sampling.lost {
            self.texts.get("long-screenshot-lost")
        } else {
            self.texts.format(
                "long-screenshot-size",
                &[
                    ("width", &sampling.long.width()),
                    ("height", &sampling.long.height()),
                ],
            )
        };
        items.push(OverlayItem::Label(self.status_label(preview, border, text)));
        let bar = self.bar(&SAMPLING_BAR, sampling.region);
        items.extend(bar.items_with(|_| false, None, self.hovered));
        OverlayScene {
            items,
            cursor: OverlayCursor::Arrow,
            text_caret: None,
        }
    }

    /// 预览下方的状态标签；下方放不下时放在预览底部的内侧。
    fn status_label(&self, preview: Rect, border: u32, text: String) -> Label {
        let display = preview::display_of(preview, &self.displays);
        let scale = display.map_or(1.0, |d| d.scale_factor);
        let font_size = FONT_SIZE * scale as f32;
        let padding = (LABEL_PADDING * scale).round() as u32;
        let gap = (LABEL_GAP * scale).round() as i32 + border as i32;
        let bottom = preview.y + preview.height as i32;
        let height = (f64::from(font_size) * 1.4).ceil() as i32 + 2 * padding as i32;
        let fits =
            display.is_none_or(|d| bottom + gap + height <= d.bounds.y + d.bounds.height as i32);
        let (anchor, corner) = if fits {
            (Point::new(preview.x, bottom + gap), Corner::TopLeft)
        } else {
            (
                Point::new(preview.x + gap, bottom - gap),
                Corner::BottomLeft,
            )
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
}

/// 控制条对一次输入的反应。
enum ControlInput {
    /// 点击了按钮。
    Clicked(Control),
    /// 落在控制条上，不再交给别人。
    Consumed,
    /// 与控制条无关。
    Pass,
}

/// `region` 里避开 `obstacles` 的部分：盖在区域里的障碍从它所在的那一边切掉（障碍靠下就
/// 切掉下方，否则靠右就切掉右侧，否则切掉左侧）。切完不剩时返回 `region` 本身。
fn clear_of(region: Rect, obstacles: &[Rect]) -> Rect {
    let mut open = region;
    for obstacle in obstacles {
        let Some(overlap) = open.intersection(*obstacle) else {
            continue;
        };
        let (left, top) = (open.x, open.y);
        let (right, bottom) = (left + open.width as i32, top + open.height as i32);
        let center_y = top + open.height as i32 / 2;
        let center_x = left + open.width as i32 / 2;
        let cut = if overlap.y >= center_y {
            (left, top, right, overlap.y)
        } else if overlap.x >= center_x {
            (left, top, overlap.x, bottom)
        } else {
            (overlap.x + overlap.width as i32, top, right, bottom)
        };
        let (l, t, r, b) = cut;
        if l < r && t < b {
            open = Rect::new(l, t, (r - l) as u32, (b - t) as u32);
        }
    }
    open
}

/// `rect` 去掉 `hole` 后剩下的部分（最多四块）。
fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
    let Some(hole) = rect.intersection(hole) else {
        return vec![rect];
    };
    let (left, top) = (rect.x, rect.y);
    let (right, bottom) = (left + rect.width as i32, top + rect.height as i32);
    let (h_left, h_top) = (hole.x, hole.y);
    let (h_right, h_bottom) = (h_left + hole.width as i32, h_top + hole.height as i32);
    [
        (left, top, right, h_top),
        (left, h_bottom, right, bottom),
        (left, h_top, h_left, h_bottom),
        (h_right, h_top, right, h_bottom),
    ]
    .into_iter()
    .filter(|&(l, t, r, b)| l < r && t < b)
    .map(|(l, t, r, b)| Rect::new(l, t, (r - l) as u32, (b - t) as u32))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_leaves_the_parts_around_the_hole() {
        let rect = Rect::new(0, 0, 10, 10);
        let parts = subtract(rect, Rect::new(3, 3, 4, 4));
        let area: u32 = parts.iter().map(|r| r.width * r.height).sum();
        assert_eq!(area, 100 - 16);
        assert_eq!(subtract(rect, Rect::new(20, 20, 2, 2)), vec![rect]);
    }

    #[test]
    fn clear_of_cuts_obstacles_off_the_side_they_sit_on() {
        let region = Rect::new(0, 0, 100, 100);
        assert_eq!(
            clear_of(region, &[Rect::new(60, 90, 40, 10)]),
            Rect::new(0, 0, 100, 90)
        );
        assert_eq!(
            clear_of(region, &[Rect::new(70, 0, 30, 40)]),
            Rect::new(0, 0, 70, 100)
        );
        assert_eq!(clear_of(region, &[Rect::new(200, 0, 5, 5)]), region);
    }
}
