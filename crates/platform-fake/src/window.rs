use std::sync::{Arc, Mutex};

use bps_platform::{
    Image, PlatformError, Rect, TopLevelWindow, WindowId, WindowLayout, WindowSource,
};

/// 一个假的顶层窗口：标题、屏幕上的外框、里面的控件，以及截取它时得到的完整内容。
#[derive(Debug, Clone)]
pub struct FakeWindow {
    title: String,
    bounds: Rect,
    controls: Vec<Rect>,
    image: Image,
}

impl FakeWindow {
    /// 外框为 `bounds`（虚拟桌面物理像素）的窗口，内容是 [`window_pattern`]`(bounds 的尺寸)`。
    pub fn new(title: &str, bounds: Rect) -> Self {
        Self {
            title: title.to_owned(),
            bounds,
            controls: Vec::new(),
            image: window_pattern(bounds.width, bounds.height),
        }
    }

    /// 自定义窗口内容（截取窗口时得到的图像）；尺寸必须等于外框的尺寸。
    pub fn with_image(mut self, image: Image) -> Self {
        assert_eq!(
            (image.width(), image.height()),
            (self.bounds.width, self.bounds.height),
            "假窗口的内容尺寸必须等于它的外框尺寸"
        );
        self.image = image;
        self
    }

    /// 在窗口里加一个控件（外框为 `bounds`，虚拟桌面物理像素）。
    pub fn with_control(mut self, bounds: Rect) -> Self {
        self.controls.push(bounds);
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 截取这个窗口时得到的图像，可作为断言的期望值。
    pub fn image(&self) -> &Image {
        &self.image
    }
}

/// 合成的窗口内容：每个像素的颜色由它在窗口内的坐标决定，蓝色通道固定为 0x5A，
/// 与 [`crate::desktop_pattern`] 不同，便于区分截到的是窗口本身还是冻结帧。
pub fn window_pattern(width: u32, height: u32) -> Image {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x * 3) as u8, (y * 5) as u8, 0x5A, 255]);
        }
    }
    Image::from_rgba(width, height, pixels)
}

/// 假桌面上的窗口，按添加顺序排列（后添加的在上层）。已关闭的窗口留空位，标识不复用。
#[derive(Default)]
pub(crate) struct WindowState {
    pub windows: Vec<Option<FakeWindow>>,
}

impl WindowState {
    fn open_windows(&self) -> impl Iterator<Item = (WindowId, &FakeWindow)> {
        self.windows
            .iter()
            .enumerate()
            .filter_map(|(index, w)| w.as_ref().map(|w| (id_of(index), w)))
    }

    fn find(&self, id: WindowId) -> Result<&FakeWindow, PlatformError> {
        self.open_windows()
            .find(|(i, _)| *i == id)
            .map(|(_, w)| w)
            .ok_or_else(|| PlatformError::new(format!("窗口 {id:?} 已关闭")))
    }
}

/// 第 `index` 个添加的窗口的标识。
pub(crate) fn id_of(index: usize) -> WindowId {
    WindowId(index as u64 + 1)
}

pub(crate) struct FakeWindowSource(pub Arc<Mutex<WindowState>>);

impl WindowSource for FakeWindowSource {
    fn layout(&self) -> Result<WindowLayout, PlatformError> {
        let state = self.0.lock().unwrap();
        let mut windows: Vec<TopLevelWindow> = state
            .open_windows()
            .map(|(id, w)| TopLevelWindow {
                id,
                title: w.title.clone(),
                bounds: w.bounds,
            })
            .collect();
        windows.reverse();
        Ok(WindowLayout::new(windows))
    }

    fn controls(&self, window: WindowId) -> Result<Vec<Rect>, PlatformError> {
        let state = self.0.lock().unwrap();
        Ok(state.find(window)?.controls.clone())
    }

    fn capture_window(&self, window: WindowId) -> Result<Image, PlatformError> {
        let state = self.0.lock().unwrap();
        Ok(state.find(window)?.image.clone())
    }
}
