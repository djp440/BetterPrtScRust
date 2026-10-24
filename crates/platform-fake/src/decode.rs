//! 把剪贴板和假文件系统里的字节解码回图像（以及假编码器写出的视频），用来在测试里验证
//! 数据真的能被读回。
//!
//! 解码器是独立于 `bps-core` 编码器的实现，期望值因此不由被测代码推导。
//! 格式不合法时一律 panic（测试失败）。

use std::io::Cursor;
use std::time::Duration;

use bps_platform::{Image, VideoFormat};

use crate::video::FAKE_VIDEO_MAGIC;

/// 解码 8 位 RGB / RGBA PNG。
pub fn png(bytes: &[u8]) -> Image {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("PNG 头无法解析");
    let mut buf = vec![0; reader.output_buffer_size().expect("PNG 尺寸过大")];
    let info = reader.next_frame(&mut buf).expect("PNG 数据无法解码");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "只支持 8 位 PNG");
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => rgb_to_rgba(data),
        other => panic!("不支持的 PNG 颜色类型 {other:?}"),
    };
    Image::from_rgba(info.width, info.height, rgba)
}

/// 解码 JPEG（有损，像素只能与原图近似比较）。
pub fn jpeg(bytes: &[u8]) -> Image {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(bytes, options);
    let rgba = decoder.decode().expect("JPEG 数据无法解码");
    let (width, height) = decoder.dimensions().expect("JPEG 缺少尺寸");
    Image::from_rgba(u32::from(width as u16), u32::from(height as u16), rgba)
}

/// 解码 WebP（有损或无损）。
pub fn webp(bytes: &[u8]) -> Image {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).expect("WebP 头无法解析");
    let (width, height) = decoder.dimensions();
    let mut buf = vec![0; decoder.output_buffer_size().expect("WebP 尺寸过大")];
    decoder.read_image(&mut buf).expect("WebP 数据无法解码");
    let rgba = if decoder.has_alpha() {
        buf
    } else {
        rgb_to_rgba(&buf)
    };
    Image::from_rgba(width, height, rgba)
}

/// 解析 AVIF 容器和其中 AV1 码流的序列头，返回图像的宽高。
///
/// 只验证结构，不解码像素（纯 Rust 的 AV1 解码器太重，不值得为测试引入）。
pub fn avif_dimensions(bytes: &[u8]) -> (u32, u32) {
    let avif = avif_parse::read_avif(&mut Cursor::new(bytes)).expect("AVIF 容器无法解析");
    let metadata = avif
        .primary_item_metadata()
        .expect("AVIF 主图像的 AV1 序列头无法解析");
    (
        metadata.max_frame_width.get(),
        metadata.max_frame_height.get(),
    )
}

/// 解码 `BITMAPINFOHEADER` + 未压缩 24/32 位像素的 DIB，支持自下而上和自上而下两种行序。
pub fn dib(bytes: &[u8]) -> Image {
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

/// 假编码器写出的一段视频。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeVideo {
    pub format: VideoFormat,
    pub width: u32,
    pub height: u32,
    /// 每秒帧数。
    pub frame_rate: u32,
    /// 按时间戳排列的全部帧。
    pub frames: Vec<FakeVideoFrame>,
}

impl FakeVideo {
    /// 视频的时长：最后一帧的时间戳再加一帧的时长。
    pub fn duration(&self) -> Duration {
        self.frames.last().map_or(Duration::ZERO, |last| {
            last.timestamp + Duration::from_secs(1) / self.frame_rate
        })
    }
}

/// 视频里的一帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeVideoFrame {
    /// 从视频开头算起的显示时间。
    pub timestamp: Duration,
    pub image: Image,
}

/// 解码假编码器（`FakePlatform` 的视频编码器）写出的视频文件。
pub fn video(bytes: &[u8]) -> FakeVideo {
    assert!(
        bytes.starts_with(FAKE_VIDEO_MAGIC),
        "不是假编码器写出的视频（长度 {}）",
        bytes.len()
    );
    let mut rest = &bytes[FAKE_VIDEO_MAGIC.len()..];
    let mut take = |n: usize| {
        assert!(rest.len() >= n, "假视频数据不完整");
        let (head, tail) = rest.split_at(n);
        rest = tail;
        head
    };
    let format = match take(4) {
        b"mp4\0" => VideoFormat::Mp4,
        b"gif\0" => VideoFormat::Gif,
        other => panic!("未知的假视频格式 {other:?}"),
    };
    let mut u32_field = || u32::from_le_bytes(take(4).try_into().unwrap());
    let (width, height, frame_rate, count) = (u32_field(), u32_field(), u32_field(), u32_field());
    let frame_bytes = width as usize * height as usize * 4;
    let frames = (0..count)
        .map(|_| {
            let nanos = u64::from_le_bytes(take(8).try_into().unwrap());
            FakeVideoFrame {
                timestamp: Duration::from_nanos(nanos),
                image: Image::from_rgba(width, height, take(frame_bytes).to_vec()),
            }
        })
        .collect();
    assert!(rest.is_empty(), "假视频末尾有多余数据");
    FakeVideo {
        format,
        width,
        height,
        frame_rate,
        frames,
    }
}

fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}
