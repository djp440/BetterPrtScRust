use std::fmt;

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
