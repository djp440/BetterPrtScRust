use std::fmt;

use crate::Rect;

/// 内存中的位图：RGBA8、非预乘 alpha、行从上到下、行间无填充。
///
/// 平台实现负责把系统给出的像素格式（如 BGRA、自下而上）转换成这种格式。
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Image {
    /// `pixels` 的长度必须等于 `width * height * 4`。
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(
            pixels.len(),
            width as usize * height as usize * 4,
            "RGBA 像素长度与 {width}x{height} 不符"
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// 全部像素，按行从上到下排列，每像素 4 字节 RGBA。
    pub fn rgba(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_rgba(self) -> Vec<u8> {
        self.pixels
    }

    /// `(x, y)` 处像素的 `[r, g, b, a]`。
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "像素坐标越界");
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// 图像中 `area`（图像内像素坐标）这一块；`area` 超出图像时返回 `None`。
    pub fn crop(&self, area: Rect) -> Option<Image> {
        let inside = area.x >= 0
            && area.y >= 0
            && area.x as u64 + u64::from(area.width) <= u64::from(self.width)
            && area.y as u64 + u64::from(area.height) <= u64::from(self.height);
        if !inside {
            return None;
        }
        let row_bytes = area.width as usize * 4;
        let stride = self.width as usize * 4;
        let mut pixels = Vec::with_capacity(row_bytes * area.height as usize);
        for row in 0..area.height as usize {
            let start = (area.y as usize + row) * stride + area.x as usize * 4;
            pixels.extend_from_slice(&self.pixels[start..start + row_bytes]);
        }
        Some(Image::from_rgba(area.width, area.height, pixels))
    }
}

impl fmt::Debug for Image {
    // 不打印像素，避免断言失败时输出几十 MB。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
