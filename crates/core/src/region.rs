//! 区域捕获：拍冻结帧 → 显示选区遮罩 → 用户框选 → 从冻结帧裁剪拼接 → 截后动作。
//!
//! 区域模式下遮罩上也能单击选中顶层窗口（ADR 0008）：截取这个窗口的完整内容
//! （`WindowSource::capture_window`，被遮挡的部分也在内，不从冻结帧裁剪）。截后动作为复制到
//! 剪贴板（以及直达 OCR）时单击即完成；等待确认时窗口截图（太大时缩小）显示在遮罩上供标注，
//! 工具条转入录屏时录这个窗口。
//!
//! 一次区域捕获跨越多个平台事件（用户的每次输入都是一个事件），所以它是常驻实例持有的
//! 一个进行中的会话，而不是一次函数调用：[`RegionCapture::start`] 打开遮罩后立即返回，
//! 之后每个 [`OverlayInput`] 交给 [`RegionCapture::handle`]，直到用户确定或取消。
//!
//! 截后动作为等待确认时，输入先交给标注（`annotation::session::Annotator`：工具条、画标注、
//! 选择结束方式、转入录屏 / 长截屏 / OCR / 翻译），标注不处理的再交给选区状态机；
//! 捕获以用户选择的结束方式结束。
//! 选好工具开始标注之前，标注把手柄、方向键等输入都交给选区状态机，选区仍可调整，工具条随
//! 调整后的选区重新定位；开始标注后仍可拖动边框，调整与标注共用撤销历史。
//!
//! 放大镜（`magnifier`）画在最上层；何时显示、对准哪个像素由选区状态机决定。

use bps_platform::{
    Backdrop, Display, Image, IncomingRequest, LocalDateTime, OverlayInput, OverlayScene, Platform,
    PlatformError, Point, Rect, TopLevelWindow,
};

use crate::annotation::finish::{self, Origin};
use crate::annotation::session::{Annotator, Finish, Response, Switch};
use crate::capture::{CaptureFailure, Screenshot, display_at};
use crate::long_screenshot::annotation_view;
use crate::magnifier::Magnifier;
use crate::recording::Target as RecordTarget;
use crate::selection::{RegionSelection, Step};
use crate::snap::Snapping;
use crate::stitch::{DesktopImage, stitch, virtual_desktop_bounds};
use crate::{
    CaptureKind, CaptureMode, CaptureRequest, CoreError, DirectFunction, PostCaptureAction, Texts,
    action,
};

/// 一次捕获是怎么结束的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ending {
    /// 截图已交付（复制、自动保存、另存为、交给结果面板）。
    Delivered,
    /// 用户取消了，没有任何产物。
    Cancelled,
    /// 用户在工具条上转入了录屏或长截屏：遮罩已关闭，由常驻实例开始新的会话接手
    /// （等待中的 CLI 请求随之转给新会话）。
    Switch(Handoff),
}

/// 转入新会话所需的信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handoff {
    /// 原来的捕获请求：新会话沿用它的输出选项、自动保存开关，再按同一热键可停止录屏。
    pub request: CaptureRequest,
    pub to: HandoffTo,
}

/// 转入哪个功能。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HandoffTo {
    /// 以这个对象开始录屏。
    Recording(RecordTarget),
    /// 以这块区域开始长截屏。
    LongScreenshot(Rect),
}

/// 进行中的区域捕获。
pub(crate) struct RegionCapture {
    request: CaptureRequest,
    /// 拍下冻结帧的时间，即这次捕获发生的时间。
    captured_at: LocalDateTime,
    /// 冻结帧覆盖的显示器。
    displays: Vec<Display>,
    selection: RegionSelection,
    /// 截后动作为等待确认时的标注界面；否则为 `None`。
    annotator: Option<Annotator>,
    /// 单击选中、正在标注的窗口：命名模板用它的标题，转入录屏、长截屏时用它。
    /// 框选时光标旁的放大镜。
    magnifier: Magnifier,
    /// 遮罩上当前画着的场景；场景没变时不重画。
    drawn: Option<OverlayScene>,
    /// 开始捕获时的界面语言：遮罩上的提示和截后动作的通知都用它。
    texts: &'static Texts,
    /// 经 CLI 转交来、等捕获结束再回复的请求。
    pub requester: Option<IncomingRequest>,
}

impl RegionCapture {
    /// 拍下冻结帧并显示选区遮罩；遮罩上的提示和截后动作的通知用 `texts` 的语言。
    pub fn start(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
        shortcuts: crate::config::UiShortcuts,
    ) -> Result<Self, CoreError> {
        let captured_at = platform.clock.now();
        // 用户要框选时，选区吸附到拍冻结帧那一刻的窗口边缘，也能单击选中窗口。
        let user_frames = request.mode == CaptureMode::Region;
        let snapping = user_frames.then(|| Snapping::of_desktop(platform.window_source.as_ref()));
        let cursor = platform.frame_source.cursor_position().ok();
        let displays = platform.overlay.open(Backdrop::FrozenFrame)?;
        let hint = texts.get("overlay-selected-hint");
        let Some(mut selection) = RegionSelection::new(displays.clone(), hint) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        if let Some(snapping) = snapping {
            selection = selection
                .with_snapping(snapping)
                .pick_windows(cursor, platform.overlay.drag_threshold());
        }
        let mut annotator = request.awaits_confirmation().then(|| {
            Annotator::new(displays.clone(), texts)
                .with_handoffs()
                .with_shortcuts(shortcuts)
        });
        // 复制到剪贴板的截图和直达 OCR：框选松开即完成，不等 Enter（录屏、长截屏仍要确认，
        // 方便开始前微调选区）。
        if matches!(
            request.kind,
            CaptureKind::Screenshot(PostCaptureAction::CopyToClipboard)
                | CaptureKind::Function(DirectFunction::Ocr)
        ) {
            selection = selection.confirm_on_release();
        }
        if let Some(annotator) = annotator.as_mut() {
            selection = selection.without_selected_hint();
            // 显示器、虚拟桌面模式不用框选：区域就是整块显示器或整个虚拟桌面，直接开始标注。
            let preset = match request.mode {
                CaptureMode::Display => {
                    let cursor = platform.frame_source.cursor_position();
                    cursor
                        .ok()
                        .and_then(|c| display_at(&displays, c))
                        .map(|d| d.bounds)
                }
                CaptureMode::VirtualDesktop => virtual_desktop_bounds(&displays),
                CaptureMode::Region => None,
            };
            if let Some(region) = preset {
                selection = selection.with_selected(region);
                if let Err(e) = annotator.lock(platform, region) {
                    platform.overlay.close();
                    return Err(e.into());
                }
            }
        }
        let mut capture = Self {
            request,
            captured_at,
            magnifier: Magnifier::new(displays.clone(), texts),
            displays,
            selection,
            annotator,
            drawn: None,
            texts,
            requester: None,
        };
        if let Err(e) = capture.redraw(platform) {
            platform.overlay.close();
            return Err(e.into());
        }
        Ok(capture)
    }

    /// 处理用户在遮罩上的一次输入。返回 `None` 表示还在框选；否则这次捕获已经结束，遮罩已关闭。
    pub fn handle(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let locked = self
            .annotator
            .as_ref()
            .is_some_and(|a| a.locked_region().is_some());
        let start_resize = self.annotator.is_some()
            && matches!(input, OverlayInput::PointerPressed { button: bps_platform::PointerButton::Left, position, .. } if self.selection.resize_hit(*position));
        if locked || start_resize {
            self.selection
                .snapping_mut()
                .load_controls(input, platform.window_source.as_ref());
            let start = start_resize;
            let resizing = self.selection.resizing();
            let pointer_move = matches!(input, OverlayInput::PointerMoved { .. });
            let pointer_event = matches!(
                input,
                OverlayInput::PointerMoved { .. }
                    | OverlayInput::PointerReleased {
                        button: bps_platform::PointerButton::Left,
                        ..
                    }
            );
            if start || (resizing && pointer_event) || pointer_move {
                if start {
                    let annotator = self.annotator.as_mut()?;
                    if let Err(error) =
                        annotator.lock_for_resize(platform, self.selection.region()?)
                    {
                        platform.overlay.close();
                        return Some(Err(CaptureFailure::before_actions(error)));
                    }
                    annotator.begin_resize();
                }
                let selection_input = match *input {
                    OverlayInput::PointerPressed {
                        button,
                        position,
                        modifiers,
                        ..
                    } if start => OverlayInput::PointerPressed {
                        button,
                        position,
                        modifiers,
                        double_click: false,
                    },
                    other => other,
                };
                self.selection.handle(&selection_input);
                if start || resizing {
                    if let Some(region) = self.selection.region() {
                        let finished = matches!(
                            input,
                            OverlayInput::PointerReleased {
                                button: bps_platform::PointerButton::Left,
                                ..
                            }
                        );
                        if let Err(error) =
                            self.annotator.as_mut()?.resize(platform, region, finished)
                        {
                            platform.overlay.close();
                            return Some(Err(CaptureFailure::before_actions(error)));
                        }
                    }
                    return self.keep_going(platform);
                }
            }
        }
        if let Some(annotator) = self.annotator.as_mut() {
            match annotator.handle(platform, input, self.selection.selected()) {
                Ok(Response::Pass) => {}
                Ok(Response::Handled) => {
                    if !self.selection.resizing() {
                        if let Some(region) = annotator.locked_region() {
                            self.selection.set_selected(region);
                        }
                    }
                    return self.keep_going(platform);
                }
                Ok(Response::Finish(finish)) => return self.finish_annotation(platform, finish),
                Ok(Response::Switch(switch)) => return self.switch(platform, switch),
                Ok(Response::Cancel) => {
                    platform.overlay.close();
                    return Some(Ok(Ending::Cancelled));
                }
                Err(e) => {
                    platform.overlay.close();
                    return Some(Err(CaptureFailure::before_actions(e)));
                }
            }
        }
        // 指针第一次进入某个窗口时取它的控件，供选区吸附到控件边缘。
        self.selection
            .snapping_mut()
            .load_controls(input, platform.window_source.as_ref());
        match self.selection.handle(input) {
            Step::Continue => {
                if let Some(annotator) = self.annotator.as_mut() {
                    if annotator.selection_editable() {
                        if self.selection.resizing() {
                            annotator.begin_resize();
                        } else if let Some(region) = self.selection.region() {
                            if Some(region) != annotator.locked_region() {
                                annotator.begin_resize();
                                if let Err(error) = annotator.resize(platform, region, true) {
                                    platform.overlay.close();
                                    return Some(Err(CaptureFailure::before_actions(error)));
                                }
                            }
                        } else {
                            annotator.clear_selection();
                        }
                    }
                }
                self.keep_going(platform)
            }
            // 等待确认时 Enter 或双击选区等于点“确认”。
            Step::Confirmed(_) if self.annotator.is_some() => {
                self.finish_annotation(platform, Finish::Confirm)
            }
            Step::Confirmed(region) => {
                Some(self.finish(platform, region).map(|()| Ending::Delivered))
            }
            Step::PickedWindow => {
                let window = self.selection.picked_window()?.clone();
                if self.annotator.is_some() {
                    self.annotate_window(platform, window)
                } else {
                    Some(
                        self.finish_window(platform, &window)
                            .map(|()| Ending::Delivered),
                    )
                }
            }
            Step::Cancelled => {
                platform.overlay.close();
                Some(Ok(Ending::Cancelled))
            }
            Step::CopyColor(focus) => match self.magnifier.copy_color(platform, focus) {
                Ok(()) => self.keep_going(platform),
                Err(e) => {
                    platform.overlay.close();
                    Some(Err(CaptureFailure::before_actions(e)))
                }
            },
        }
    }

    /// 等待确认时单击选中了 `window`：截取它的完整内容，显示在遮罩上开始标注。
    fn annotate_window(
        &mut self,
        platform: &mut Platform,
        window: TopLevelWindow,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let image = match platform.window_source.capture_window(window.id) {
            Ok(image) => image,
            Err(e) => {
                platform.overlay.close();
                return Some(Err(CaptureFailure::before_actions(e)));
            }
        };
        let view = annotation_view(window.bounds, &image, &self.displays);
        self.annotator.as_mut()?.lock_image(view, image);
        self.selection.set_selected(view);
        self.annotator.as_mut()?.set_window(window);
        self.keep_going(platform)
    }

    /// 关闭遮罩，截取 `window` 的完整内容，再执行截后动作。
    fn finish_window(
        &mut self,
        platform: &mut Platform,
        window: &TopLevelWindow,
    ) -> Result<(), CaptureFailure> {
        platform.overlay.close();
        let image = platform
            .window_source
            .capture_window(window.id)
            .map_err(CaptureFailure::before_actions)?;
        let screenshot = Screenshot {
            image,
            mode: CaptureMode::Region,
            captured_at: self.captured_at,
            window_title: Some(window.title.clone()),
        };
        action::run(platform, &screenshot, &self.request, self.texts)
    }

    /// 捕获还在进行：重画遮罩。重画失败时关闭遮罩，捕获失败。
    fn keep_going(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        match self.redraw(platform) {
            Ok(()) => None,
            Err(e) => {
                platform.overlay.close();
                Some(Err(CaptureFailure::before_actions(e)))
            }
        }
    }

    /// 用户在工具条上转入了另一个功能。OCR、翻译把（含标注的）截图交给结果面板；录屏、长截屏
    /// 关闭遮罩，由常驻实例以当前选区开始新的会话（已画的标注不带过去）。标注的是单击选中的
    /// 窗口时，录屏录这个窗口，长截屏取它当前在屏幕上的范围。
    fn switch(
        &mut self,
        platform: &mut Platform,
        switch: Switch,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let annotator = self.annotator.as_ref()?;
        let region = annotator
            .locked_region()
            .or_else(|| self.selection.selected())?;
        let to = match switch {
            Switch::Ocr | Switch::Translate => {
                let translate = switch == Switch::Translate;
                let done = finish::recognize(
                    platform,
                    annotator,
                    region,
                    translate,
                    self.request.ocr_auto_copy,
                    self.texts,
                );
                return Some(done.map(|()| Ending::Delivered));
            }
            Switch::Record => HandoffTo::Recording(match annotator.window() {
                Some(window) => RecordTarget::Window(window.clone()),
                None => RecordTarget::Region(region),
            }),
            Switch::LongScreenshot => HandoffTo::LongScreenshot(
                annotator
                    .window()
                    .and_then(|window| {
                        virtual_desktop_bounds(&self.displays)?.intersection(window.bounds)
                    })
                    .unwrap_or(region),
            ),
        };
        platform.overlay.close();
        Some(Ok(Ending::Switch(Handoff {
            request: self.request.clone(),
            to,
        })))
    }

    /// 用户选择了标注的结束方式：取得合成了标注的截图，关闭遮罩，按结束方式输出。
    /// 另存为时先弹出对话框（遮罩还在），用户取消就回到标注。
    fn finish_annotation(
        &mut self,
        platform: &mut Platform,
        finish: Finish,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let annotator = self.annotator.as_ref()?;
        let region = annotator
            .locked_region()
            .or_else(|| self.selection.selected())?;
        match finish::conclude(
            platform,
            annotator,
            region,
            &self.request,
            &Origin {
                captured_at: self.captured_at,
                window_title: annotator.window().map(|w| w.title.clone()),
            },
            finish,
            self.texts,
        ) {
            None => self.keep_going(platform),
            Some(result) => Some(result.map(|()| Ending::Delivered)),
        }
    }

    /// 从冻结帧裁剪拼接出 `region` 的截图，关闭遮罩，再执行截后动作。
    fn finish(
        &mut self,
        platform: &mut Platform,
        region: bps_platform::Rect,
    ) -> Result<(), CaptureFailure> {
        let image = frozen_region(platform, &self.displays, region);
        platform.overlay.close();
        let screenshot = Screenshot {
            image: image.map_err(CaptureFailure::before_actions)?,
            mode: CaptureMode::Region,
            captured_at: self.captured_at,
            window_title: None,
        };
        action::run(platform, &screenshot, &self.request, self.texts)
    }

    fn redraw(&mut self, platform: &mut Platform) -> Result<(), PlatformError> {
        // 开始标注后隐藏放大镜和选区内部移动指针；边框手柄继续参与调整。
        if self
            .annotator
            .as_ref()
            .is_some_and(|a| a.locked_region().is_some() && !a.selection_editable())
        {
            self.selection.lock();
        }
        let mut scene = self.selection.scene();
        let mut canvas_changed = false;
        if let Some(annotator) = self.annotator.as_mut() {
            canvas_changed = annotator.sync_canvas(platform.overlay.as_mut())?;
            let cursor = scene.cursor;
            annotator.decorate(&mut scene, self.selection.selected());
            if matches!(
                cursor,
                bps_platform::OverlayCursor::ResizeEw
                    | bps_platform::OverlayCursor::ResizeNs
                    | bps_platform::OverlayCursor::ResizeNwse
                    | bps_platform::OverlayCursor::ResizeNesw
            ) {
                scene.cursor = cursor;
            }
        }
        if let Some(focus) = self.selection.magnifier_focus() {
            let magnifier = self.magnifier.items(platform.overlay.as_ref(), focus)?;
            scene.items.extend(magnifier);
        }
        if canvas_changed || self.drawn.as_ref() != Some(&scene) {
            platform.overlay.render(&scene)?;
            self.drawn = Some(scene);
        }
        Ok(())
    }
}

/// 冻结帧中 `region` 的画面：从与它重叠的各显示器的冻结帧裁剪拼接，没有显示器覆盖的部分透明。
/// 遮罩必须还开着。
pub(crate) fn frozen_region(
    platform: &Platform,
    displays: &[Display],
    region: Rect,
) -> Result<Image, PlatformError> {
    let parts = displays
        .iter()
        .filter(|display| display.bounds.intersection(region).is_some())
        .map(|display| {
            Ok(DesktopImage {
                origin: Point::new(display.bounds.x, display.bounds.y),
                image: platform.overlay.frozen_image(display.id)?,
            })
        })
        .collect::<Result<Vec<_>, PlatformError>>()?;
    Ok(stitch(region, &parts))
}
