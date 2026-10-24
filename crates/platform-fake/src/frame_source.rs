use std::sync::{Arc, Mutex};

use bps_platform::{Display, DisplayId, FrameSource, Image, PlatformError, Point, Rect};

/// 一块假显示器：位置、缩放比例，以及它“显示”的画面。
#[derive(Debug, Clone)]
pub struct FakeDisplay {
    bounds: Rect,
    scale_factor: f64,
    is_primary: bool,
    image: Image,
}

impl FakeDisplay {
    /// 位于 `bounds`（物理像素）的显示器，缩放比例 1.0，画面是 [`desktop_pattern`]`(bounds)`。
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            scale_factor: 1.0,
            is_primary: false,
            image: desktop_pattern(bounds),
        }
    }

    pub fn primary(mut self) -> Self {
        self.is_primary = true;
        self
    }

    pub fn scale_factor(mut self, scale_factor: f64) -> Self {
        self.scale_factor = scale_factor;
        self
    }

    /// 自定义画面；尺寸必须等于 `bounds` 的物理像素尺寸。
    pub fn with_image(mut self, image: Image) -> Self {
        assert_eq!(
            (image.width(), image.height()),
            (self.bounds.width, self.bounds.height),
            "假显示器的画面尺寸必须等于它的物理像素尺寸"
        );
        self.image = image;
        self
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 这块显示器当前的画面，可作为断言的期望值。
    pub fn image(&self) -> &Image {
        &self.image
    }
}

/// 合成的“桌面”画面：虚拟桌面上每个物理像素都有由坐标决定的唯一颜色。
///
/// 每块 [`FakeDisplay`] 默认显示它所在范围的这幅画面，因此任意区域的期望截图
/// 都可以直接用 `desktop_pattern(区域)` 得到，与显示器如何划分无关。
pub fn desktop_pattern(area: Rect) -> Image {
    let mut pixels = Vec::with_capacity(area.width as usize * area.height as usize * 4);
    for row in 0..area.height {
        let y = area.y + row as i32;
        for col in 0..area.width {
            let x = area.x + col as i32;
            let r = x as u8;
            let g = y as u8;
            let b = ((x >> 8) as u8).wrapping_mul(16) ^ ((y >> 8) as u8);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Image::from_rgba(area.width, area.height, pixels)
}

#[derive(Default)]
pub(crate) struct FrameState {
    pub displays: Vec<FakeDisplay>,
    pub cursor: Point,
}

pub(crate) struct FakeFrameSource(pub Arc<Mutex<FrameState>>);

fn id_of(index: usize) -> DisplayId {
    DisplayId(index as u64 + 1)
}

impl FrameSource for FakeFrameSource {
    fn displays(&self) -> Result<Vec<Display>, PlatformError> {
        let state = self.0.lock().unwrap();
        Ok(state
            .displays
            .iter()
            .enumerate()
            .map(|(index, d)| Display {
                id: id_of(index),
                bounds: d.bounds,
                scale_factor: d.scale_factor,
                is_primary: d.is_primary,
            })
            .collect())
    }

    fn cursor_position(&self) -> Result<Point, PlatformError> {
        Ok(self.0.lock().unwrap().cursor)
    }

    fn capture_display(&self, display: DisplayId) -> Result<Image, PlatformError> {
        let state = self.0.lock().unwrap();
        state
            .displays
            .iter()
            .enumerate()
            .find(|(index, _)| id_of(*index) == display)
            .map(|(_, d)| d.image.clone())
            .ok_or_else(|| PlatformError::new(format!("假帧源中没有显示器 {display:?}")))
    }
}
