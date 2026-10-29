use std::sync::{Arc, Mutex};

use bps_platform::{
    Backdrop, Display, DisplayId, Image, OverlayScene, PlatformError, Point, Rect,
    SelectionOverlay, TextMask,
};

use crate::frame_source::FrameState;

/// 假遮罩的状态：是否打开、最近一次画出的场景、打开时拍下的冻结帧。
#[derive(Default)]
pub(crate) struct OverlayState {
    open: bool,
    /// 最近一次 `render` 的场景；打开后还没画过时为 `None`（遮罩还看不见）。
    scene: Option<OverlayScene>,
    frozen: Vec<(DisplayId, Image)>,
    /// 打开时的背景。
    backdrop: Option<Backdrop>,
    /// 画布的位置和内容（包括还没 `render` 出来的更新）。
    canvas: Option<(Rect, Image)>,
    /// 最近一次 `render` 时的画布，即用户看到的画布。
    shown_canvas: Option<(Rect, Image)>,
    /// 透过遮罩的范围（这里的鼠标操作交给底下的窗口）。
    pub pass_through: Vec<Rect>,
}

impl OverlayState {
    /// 正显示着的遮罩底下是什么。
    pub fn visible_backdrop(&self) -> Option<Backdrop> {
        self.visible_scene().and(self.backdrop)
    }

    /// 遮罩是否正显示在屏幕上（打开并且至少画过一次）。
    pub fn visible_scene(&self) -> Option<&OverlayScene> {
        self.scene.as_ref().filter(|_| self.open)
    }

    /// 遮罩打开时透过遮罩的范围。
    pub fn visible_pass_through(&self) -> Vec<Rect> {
        if self.open {
            self.pass_through.clone()
        } else {
            Vec::new()
        }
    }

    /// 用户正看到的画布（最近一次 `render` 时的画布）。
    pub fn visible_canvas(&self) -> Option<&(Rect, Image)> {
        self.shown_canvas.as_ref().filter(|_| self.open)
    }
}

pub(crate) struct FakeOverlay {
    pub frames: Arc<Mutex<FrameState>>,
    pub state: Arc<Mutex<OverlayState>>,
}

impl SelectionOverlay for FakeOverlay {
    fn open(&mut self, backdrop: Backdrop) -> Result<Vec<Display>, PlatformError> {
        let mut state = self.state.lock().unwrap();
        if state.open {
            return Err(PlatformError::new("假遮罩已经打开"));
        }
        let frames = self.frames.lock().unwrap();
        let displays = frames.describe();
        if displays.is_empty() {
            return Err(PlatformError::new("假帧源中没有显示器"));
        }
        // 冻结帧：此刻每块显示器画面的副本，之后画面再变也不影响它。实时画面不拍冻结帧。
        state.frozen = match backdrop {
            Backdrop::FrozenFrame => frames
                .displays
                .iter()
                .zip(&displays)
                .map(|(fake, display)| (display.id, fake.image().clone()))
                .collect(),
            Backdrop::Live => Vec::new(),
        };
        state.backdrop = Some(backdrop);
        state.open = true;
        state.pass_through.clear();
        state.scene = None;
        state.canvas = None;
        state.shown_canvas = None;
        Ok(displays)
    }

    fn render(&mut self, scene: &OverlayScene) -> Result<(), PlatformError> {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(PlatformError::new("假遮罩没有打开"));
        }
        state.scene = Some(scene.clone());
        state.shown_canvas = state.canvas.clone();
        Ok(())
    }

    fn frozen_image(&self, display: DisplayId) -> Result<Image, PlatformError> {
        let state = self.state.lock().unwrap();
        state
            .frozen
            .iter()
            .find(|(id, _)| *id == display)
            .map(|(_, image)| image.clone())
            .ok_or_else(|| PlatformError::new(format!("冻结帧中没有显示器 {display:?}")))
    }

    fn show_canvas(&mut self, area: Rect, image: &Image) -> Result<(), PlatformError> {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(PlatformError::new("假遮罩没有打开"));
        }
        assert_eq!(
            (image.width(), image.height()),
            (area.width, area.height),
            "画布内容的尺寸必须等于画布范围"
        );
        state.canvas = Some((area, image.clone()));
        Ok(())
    }

    fn update_canvas(&mut self, at: Point, patch: &Image) -> Result<(), PlatformError> {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(PlatformError::new("假遮罩没有打开"));
        }
        let (area, image) = state
            .canvas
            .as_mut()
            .expect("更新画布之前必须先 show_canvas");
        let target = Rect::new(at.x, at.y, patch.width(), patch.height());
        assert!(
            area.intersection(target) == Some(target),
            "画布更新 {target:?} 超出了画布 {area:?}"
        );
        let (left, top) = ((at.x - area.x) as usize, (at.y - area.y) as usize);
        let width = image.width() as usize;
        let mut pixels = std::mem::replace(image, Image::from_rgba(0, 0, Vec::new())).into_rgba();
        let row_bytes = patch.width() as usize * 4;
        if row_bytes > 0 {
            for (row, source) in patch.rgba().chunks_exact(row_bytes).enumerate() {
                let start = ((top + row) * width + left) * 4;
                pixels[start..start + row_bytes].copy_from_slice(source);
            }
        }
        *image = Image::from_rgba(area.width, area.height, pixels);
        Ok(())
    }

    fn rasterize_text(&self, text: &str, font_size: u32) -> Result<TextMask, PlatformError> {
        if !self.state.lock().unwrap().open {
            return Err(PlatformError::new("假遮罩没有打开"));
        }
        Ok(crate::text::text_mask(text, font_size))
    }

    fn set_pass_through(&mut self, areas: &[Rect]) -> Result<(), PlatformError> {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(PlatformError::new("假遮罩没有打开"));
        }
        state.pass_through = areas.to_vec();
        Ok(())
    }

    fn close(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.open = false;
        state.pass_through.clear();
        state.scene = None;
        state.frozen.clear();
        state.backdrop = None;
        state.canvas = None;
        state.shown_canvas = None;
    }
}
