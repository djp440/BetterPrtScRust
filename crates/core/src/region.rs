//! 区域捕获：拍冻结帧 → 显示选区遮罩 → 用户框选 → 从冻结帧裁剪拼接 → 截后动作。
//!
//! 一次区域捕获跨越多个平台事件（用户的每次输入都是一个事件），所以它是常驻实例持有的
//! 一个进行中的会话，而不是一次函数调用：[`RegionCapture::start`] 打开遮罩后立即返回，
//! 之后每个 [`OverlayInput`] 交给 [`RegionCapture::handle`]，直到用户确定或取消。
//!
//! 截后动作为等待确认时，输入先交给标注（`annotation::session::Annotator`：工具条、画标注、
//! 选择结束方式、转入录屏 / 长截屏 / OCR / 翻译），标注不处理的再交给选区状态机；
//! 捕获以用户选择的结束方式结束。
//! 选好工具开始标注之前，标注把手柄、方向键等输入都交给选区状态机，选区仍可调整，工具条随
//! 调整后的选区重新定位；开始标注后区域锁定，选区状态机不再收到输入、不再画手柄和放大镜。
//!
//! 放大镜（`magnifier`）画在最上层；何时显示、对准哪个像素由选区状态机决定。

use bps_platform::{
    Backdrop, Display, Image, IncomingRequest, LocalDateTime, OverlayInput, OverlayScene, Platform,
    PlatformError, Point, Rect,
};

use crate::annotation::finish::{self, Origin};
use crate::annotation::session::{Annotator, Finish, Response, Switch};
use crate::capture::{CaptureFailure, Screenshot, display_at};
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
    ) -> Result<Self, CoreError> {
        let captured_at = platform.clock.now();
        // 用户要框选时，选区吸附到拍冻结帧那一刻的窗口边缘。
        let user_frames = matches!(request.mode, CaptureMode::Region | CaptureMode::Window);
        let snapping = user_frames.then(|| Snapping::of_desktop(platform.window_source.as_ref()));
        let displays = platform.overlay.open(Backdrop::FrozenFrame)?;
        let hint = texts.get("overlay-selected-hint");
        let Some(mut selection) = RegionSelection::new(displays.clone(), hint) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        if let Some(snapping) = snapping {
            selection = selection.with_snapping(snapping);
        }
        let mut annotator = request
            .awaits_confirmation()
            .then(|| Annotator::new(displays.clone(), texts).with_handoffs());
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
                CaptureMode::Region | CaptureMode::Window => None,
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
        if let Some(annotator) = self.annotator.as_mut() {
            match annotator.handle(platform, input, self.selection.selected()) {
                Ok(Response::Pass) => {}
                Ok(Response::Handled) => return self.keep_going(platform),
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
            Step::Continue => self.keep_going(platform),
            // 等待确认时 Enter 或双击选区等于点“确认”。
            Step::Confirmed(_) if self.annotator.is_some() => {
                self.finish_annotation(platform, Finish::Confirm)
            }
            Step::Confirmed(region) => {
                Some(self.finish(platform, region).map(|()| Ending::Delivered))
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
    /// 关闭遮罩，由常驻实例以当前选区开始新的会话（已画的标注不带过去）。
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
            Switch::Record => HandoffTo::Recording(RecordTarget::Region(region)),
            Switch::LongScreenshot => HandoffTo::LongScreenshot(region),
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
                window_title: None,
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
        // 开始标注后区域锁定，选区不能再调整：不再画手柄和放大镜。
        if self
            .annotator
            .as_ref()
            .is_some_and(|a| a.locked_region().is_some())
        {
            self.selection.lock();
        }
        let mut scene = self.selection.scene();
        let mut canvas_changed = false;
        if let Some(annotator) = self.annotator.as_mut() {
            canvas_changed = annotator.sync_canvas(platform.overlay.as_mut())?;
            annotator.decorate(&mut scene, self.selection.selected());
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
