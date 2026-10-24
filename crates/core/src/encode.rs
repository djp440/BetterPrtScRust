//! 把截图编码成剪贴板和磁盘使用的字节格式。

use bps_platform::Image;

use crate::{CoreError, OutputFormat};

/// JPEG 质量（1–100）。截图里文字多，取偏高的值。
const JPEG_QUALITY: u8 = 90;
/// AVIF 质量（1–100）与 rav1e 速度（1 最慢 – 10 最快）。截图在热键处理中同步编码，取最快速度。
const AVIF_QUALITY: f32 = 85.0;
const AVIF_SPEED: u8 = 10;

/// 按输出格式编码成完整的文件字节。
pub(crate) fn encode(image: &Image, format: OutputFormat) -> Result<Vec<u8>, CoreError> {
    match format {
        OutputFormat::Png => png(image),
        OutputFormat::Jpeg => jpeg(image),
        OutputFormat::WebP => webp(image),
        OutputFormat::Avif => avif(image),
    }
}

/// 编码成 JPEG。JPEG 没有透明度，半透明和透明像素先合成到白色背景上。
fn jpeg(image: &Image) -> Result<Vec<u8>, CoreError> {
    let (width, height) = (u16::try_from(image.width()), u16::try_from(image.height()));
    let (Ok(width), Ok(height)) = (width, height) else {
        return Err(CoreError::Encode(format!(
            "JPEG 最大只支持 65535x65535，截图为 {}x{}",
            image.width(),
            image.height()
        )));
    };
    let rgb: Vec<u8> = image
        .rgba()
        .chunks_exact(4)
        .flat_map(|p| {
            let alpha = u16::from(p[3]);
            let over_white =
                |c: u8| ((u16::from(c) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            [over_white(p[0]), over_white(p[1]), over_white(p[2])]
        })
        .collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, JPEG_QUALITY)
        .encode(&rgb, width, height, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| CoreError::Encode(e.to_string()))?;
    Ok(out)
}

/// 编码成无损 WebP，保留透明度。
fn webp(image: &Image) -> Result<Vec<u8>, CoreError> {
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out)
        .encode(
            image.rgba(),
            image.width(),
            image.height(),
            image_webp::ColorType::Rgba8,
        )
        .map_err(|e| CoreError::Encode(e.to_string()))?;
    Ok(out)
}

/// 编码成有损 AVIF；完全不透明时不写 alpha 通道。
fn avif(image: &Image) -> Result<Vec<u8>, CoreError> {
    let pixels: Vec<ravif::RGBA8> = image
        .rgba()
        .chunks_exact(4)
        .map(|p| ravif::RGBA8::new(p[0], p[1], p[2], p[3]))
        .collect();
    let encoded = ravif::Encoder::new()
        .with_quality(AVIF_QUALITY)
        .with_speed(AVIF_SPEED)
        .encode_rgba(ravif::Img::new(
            &pixels[..],
            image.width() as usize,
            image.height() as usize,
        ))
        .map_err(|e| CoreError::Encode(e.to_string()))?;
    Ok(encoded.avif_file)
}

/// 编码成 PNG（RGBA8）。剪贴板场景优先速度，使用快速压缩。
pub(crate) fn png(image: &Image) -> Result<Vec<u8>, CoreError> {
    let encode_error = |e: png::EncodingError| CoreError::Encode(e.to_string());
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(encode_error)?;
    writer
        .write_image_data(image.rgba())
        .map_err(encode_error)?;
    writer.finish().map_err(encode_error)?;
    Ok(out)
}

const BITMAPINFOHEADER_SIZE: u32 = 40;
const BI_RGB: u32 = 0;

/// 编码成 DIB：`BITMAPINFOHEADER`（32 位、`BI_RGB`、正高度）后接自下而上的 BGRA 像素。
///
/// 正高度（自下而上）是兼容性最好的写法；32 位每行天然 4 字节对齐，无需填充。
pub(crate) fn dib(image: &Image) -> Vec<u8> {
    let (width, height) = (image.width(), image.height());
    let row_bytes = width as usize * 4;
    let pixel_bytes = row_bytes * height as usize;

    let mut out = Vec::with_capacity(BITMAPINFOHEADER_SIZE as usize + pixel_bytes);
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes()); // biSize
    out.extend_from_slice(&(width as i32).to_le_bytes()); // biWidth
    out.extend_from_slice(&(height as i32).to_le_bytes()); // biHeight：正数表示自下而上
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&BI_RGB.to_le_bytes()); // biCompression
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes()); // biSizeImage
    out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    if row_bytes > 0 {
        for row in image.rgba().chunks_exact(row_bytes).rev() {
            for p in row.chunks_exact(4) {
                out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
    }
    out
}
