//! 把截图编码成剪贴板和磁盘使用的字节格式。

use bps_platform::Image;

use crate::CoreError;

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
