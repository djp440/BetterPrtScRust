//! 窗口捕获：取窗口布局并拍冻结帧 → 显示选区遮罩 → 用户悬停高亮、单击选中顶层窗口 →
//! 截取这个窗口的完整内容 → 截后动作。截后动作包含标注时，遮罩不关，窗口截图（太大时缩小）
//! 显示在遮罩上供标注（与长截屏的标注阶段用同一套标注会话），标注结束后再执行其余截后动作。
//!
//! 与区域捕获一样跨越多个平台事件：[`WindowCapture::start`] 打开遮罩后立即返回，之后每个
//! [`OverlayInput`] 交给 [`WindowCapture::handle`]，直到用户选中窗口（以及标注完）或取消。
//! 窗口内容来自 `WindowSource::capture_window`（被遮挡的部分也在内），不从冻结帧裁剪；
//! 冻结帧只是选择时的背景。

use bps_platform::{
    Backdrop, Display, IncomingRequest, LocalDateTime, OverlayCursor, OverlayInput, OverlayItem,
    OverlayScene, Platform, PlatformError, Point, Rect, TopLevelWindow,
};

use crate::annotation::finish::{self, Origin};
use crate::annotation::session::{Annotator, Response};
use crate::capture::{CaptureFailure, Screenshot};
use crate::long_screenshot::annotation_view;
use crate::region::Ending;
use crate::selection::{ACCENT, mask_around};
use crate::stitch::virtual_desktop_bounds;
use crate::window_selection::{WindowSelection, WindowStep};
use crate::{CaptureMode, CaptureRequest, CoreError, PostCaptureAction, Texts, action};

/// 选中窗口后的标注阶段。
struct Annotating {
    annotator: Box<Annotator>,
    /// 窗口截图（可能缩小后）显示的范围。
    view: Rect,
    /// 被选中窗口的标题，命名模板用。
    window_title: String,
}

/// 进行中的窗口捕获。
pub(crate) struct WindowCapture {
    request: CaptureRequest,
    /// 拍下冻结帧的时间，即这次捕获发生的时间。
    captured_at: LocalDateTime,
    selection: WindowSelection,
    /// 冻结帧覆盖的显示器。
    displays: Vec<Display>,
    /// 选中窗口后标注它的截图；还在选择窗口时为 `None`。
    annotating: Option<Annotating>,
    /// 遮罩上当前画着的场景；场景没变时不重画。
    drawn: Option<OverlayScene>,
    /// 开始捕获时的界面语言：截后动作的通知用它。
    texts: &'static Texts,
    /// 经 CLI 转交来、等捕获结束再回复的请求。
    pub requester: Option<IncomingRequest>,
}

impl WindowCapture {
    /// 取得此刻的窗口布局、拍下冻结帧并显示选区遮罩；截后动作的通知用 `texts` 的语言。
    pub fn start(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
    ) -> Result<Self, CoreError> {
        let captured_at = platform.clock.now();
        let layout = platform.window_source.layout()?;
        let displays = platform.overlay.open(Backdrop::FrozenFrame)?;
        // 遮罩一出现就高亮光标下的窗口；读不到光标位置时等第一次移动。
        let cursor = platform.frame_source.cursor_position().ok();
        let Some(selection) = WindowSelection::new(displays.clone(), layout, cursor) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        let mut capture = Self {
            request,
            captured_at,
            selection,
            displays,
            annotating: None,
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

    /// 处理用户在遮罩上的一次输入。返回 `None` 表示还在进行；否则这次捕获已经结束，遮罩已关闭。
    pub fn handle(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        if self.annotating.is_some() {
            return self.handle_annotating(platform, input);
        }
        match self.selection.handle(input) {
            WindowStep::Continue => self.keep_going(platform),
            WindowStep::Confirmed(window) => self.confirmed(platform, &window),
            WindowStep::Cancelled => {
                platform.overlay.close();
                Some(Ok(Ending::Cancelled))
            }
        }
    }

    /// 用户选中了 `window`：包含标注时截取它的完整内容并进入标注，否则关闭遮罩、截取并执行截后动作。
    fn confirmed(
        &mut self,
        platform: &mut Platform,
        window: &TopLevelWindow,
    ) -> Option<Result<Ending, CaptureFailure>> {
        if !self.request.actions.contains(&PostCaptureAction::Annotate) {
            return Some(self.finish(platform, window).map(|()| Ending::Delivered));
        }
        let image = match platform.window_source.capture_window(window.id) {
            Ok(image) => image,
            Err(e) => {
                platform.overlay.close();
                return Some(Err(CaptureFailure::before_actions(e)));
            }
        };
        let view = annotation_view(window.bounds, &image, &self.displays);
        let mut annotator = Annotator::new(self.displays.clone(), self.texts);
        annotator.lock_image(view, image);
        self.annotating = Some(Annotating {
            annotator: Box::new(annotator),
            view,
            window_title: window.title.clone(),
        });
        self.keep_going(platform)
    }

    fn handle_annotating(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let annotating = self.annotating.as_mut()?;
        let view = annotating.view;
        match annotating.annotator.handle(platform, input, Some(view)) {
            Ok(Response::Pass | Response::Handled) => self.keep_going(platform),
            Ok(Response::Finish(how)) => {
                let annotating = self.annotating.as_ref()?;
                let origin = Origin {
                    captured_at: self.captured_at,
                    window_title: Some(annotating.window_title.clone()),
                };
                match finish::conclude(
                    platform,
                    &annotating.annotator,
                    view,
                    &self.request,
                    &origin,
                    how,
                    self.texts,
                ) {
                    None => self.keep_going(platform),
                    Some(result) => Some(result.map(|()| Ending::Delivered)),
                }
            }
            Ok(Response::Cancel) => {
                platform.overlay.close();
                Some(Ok(Ending::Cancelled))
            }
            Err(e) => {
                platform.overlay.close();
                Some(Err(CaptureFailure::before_actions(e)))
            }
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

    /// 关闭遮罩，截取 `window` 的完整内容，再执行截后动作。
    fn finish(
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
            mode: CaptureMode::Window,
            captured_at: self.captured_at,
            window_title: Some(window.title.clone()),
        };
        action::run(platform, &screenshot, &self.request, self.texts)
    }

    fn redraw(&mut self, platform: &mut Platform) -> Result<(), PlatformError> {
        let mut canvas_changed = false;
        let scene = match self.annotating.as_mut() {
            Some(annotating) => {
                canvas_changed = annotating
                    .annotator
                    .sync_canvas(platform.overlay.as_mut())?;
                annotating_scene(annotating, &self.displays)
            }
            None => self.selection.scene(),
        };
        if canvas_changed || self.drawn.as_ref() != Some(&scene) {
            platform.overlay.render(&scene)?;
            self.drawn = Some(scene);
        }
        Ok(())
    }
}

/// 标注阶段遮罩上的内容：窗口截图以外变暗，截图加边框，再叠上标注工具条和已画的标注。
fn annotating_scene(annotating: &Annotating, displays: &[Display]) -> OverlayScene {
    let view = annotating.view;
    let mut items = virtual_desktop_bounds(displays)
        .map(|bounds| mask_around(bounds, view))
        .unwrap_or_default();
    let scale = displays
        .iter()
        .find(|d| d.bounds.contains(Point::new(view.x, view.y)))
        .map_or(1.0, |d| d.scale_factor);
    items.push(OverlayItem::Frame {
        rect: view,
        width: (scale.round() as u32).max(1),
        color: ACCENT,
    });
    let mut scene = OverlayScene {
        items,
        cursor: OverlayCursor::Arrow,
        text_caret: None,
    };
    annotating.annotator.decorate(&mut scene, Some(view));
    scene
}
