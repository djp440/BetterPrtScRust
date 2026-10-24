use std::io::Cursor;
use std::sync::{Arc, Mutex};

use bps_platform::{Clipboard, ClipboardItem, Image, PlatformError};

/// 剪贴板某一时刻的内容，提供把各格式解码回图像的辅助方法。
///
/// 解码器是独立于 `bps-core` 编码器的实现，用来在测试里验证剪贴板数据真的能被读回。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardContent {
    items: Vec<ClipboardItem>,
}

impl ClipboardContent {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[ClipboardItem] {
        &self.items
    }

    /// 剪贴板中的格式名称，按写入顺序排列，例如 `["PNG", "DIB"]`。
    pub fn formats(&self) -> Vec<&'static str> {
        self.items.iter().map(ClipboardItem::format_name).collect()
    }

    /// 把 PNG 格式解码成图像；没有 PNG 格式时返回 `None`，解码失败时 panic。
    pub fn png_image(&self) -> Option<Image> {
        self.items.iter().find_map(|item| match item {
            ClipboardItem::Png(bytes) => Some(decode_png(bytes)),
            _ => None,
        })
    }

    /// 把 DIB 格式解码成图像；没有 DIB 格式时返回 `None`，格式不合法时 panic。
    pub fn dib_image(&self) -> Option<Image> {
        self.items.iter().find_map(|item| match item {
            ClipboardItem::Dib(bytes) => Some(decode_dib(bytes)),
            _ => None,
        })
    }
}

pub(crate) struct FakeClipboard(pub Arc<Mutex<Option<ClipboardContent>>>);

impl Clipboard for FakeClipboard {
    fn write(&mut self, items: Vec<ClipboardItem>) -> Result<(), PlatformError> {
        *self.0.lock().unwrap() = Some(ClipboardContent { items });
        Ok(())
    }
}

fn decode_png(bytes: &[u8]) -> Image {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("PNG 头无法解析");
    let mut buf = vec![0; reader.output_buffer_size().expect("PNG 尺寸过大")];
    let info = reader.next_frame(&mut buf).expect("PNG 数据无法解码");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "只支持 8 位 PNG");
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("不支持的 PNG 颜色类型 {other:?}"),
    };
    Image::from_rgba(info.width, info.height, rgba)
}

/// 解码 `BITMAPINFOHEADER` + 未压缩 24/32 位像素的 DIB，支持自下而上和自上而下两种行序。
fn decode_dib(bytes: &[u8]) -> Image {
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let i32_at = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());

    let header_size = u32_at(0) as usize;
    assert_eq!(header_size, 40, "只支持 BITMAPINFOHEADER");
    let width = i32_at(4);
    let height = i32_at(8);
    let bit_count = u16_at(14);
    let compression = u32_at(16);
    assert!(width > 0, "DIB 宽度必须为正");
    assert_eq!(compression, 0, "只支持 BI_RGB");
    assert!(bit_count == 24 || bit_count == 32, "只支持 24/32 位 DIB");

    let (width, rows) = (width as usize, height.unsigned_abs() as usize);
    let bytes_per_pixel = bit_count as usize / 8;
    let stride = (width * bytes_per_pixel).div_ceil(4) * 4;
    let pixels = &bytes[header_size..];
    assert!(pixels.len() >= stride * rows, "DIB 像素数据不完整");

    let mut rgba = Vec::with_capacity(width * rows * 4);
    for y in 0..rows {
        let source_row = if height > 0 { rows - 1 - y } else { y };
        let row = &pixels[source_row * stride..][..width * bytes_per_pixel];
        for p in row.chunks_exact(bytes_per_pixel) {
            let alpha = if bytes_per_pixel == 4 { p[3] } else { 255 };
            rgba.extend_from_slice(&[p[2], p[1], p[0], alpha]);
        }
    }
    Image::from_rgba(width as u32, rows as u32, rgba)
}
