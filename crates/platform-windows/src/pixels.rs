//! 把 Desktop Duplication 的像素格式转换成 `bps_platform::Image` 的 RGBA8。

use std::sync::{Arc, Mutex, PoisonError};

use bps_platform::Image;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_ROTATE180,
    DXGI_MODE_ROTATION_ROTATE270,
};

/// 桌面纹理的像素格式。
pub(crate) enum DesktopFormat {
    /// `DXGI_FORMAT_B8G8R8A8_UNORM`，sRGB 编码。
    Bgra8,
    /// `DXGI_FORMAT_R16G16B16A16_FLOAT`：HDR / 高级颜色桌面使用的线性 scRGB，
    /// 1.0 = 80 尼特。`sdr_white` 是 SDR 白色对应的 scRGB 数值。
    ScRgbF16 { sdr_white: f32 },
}

/// 每段至少这么多行才值得开线程。
const MIN_ROWS_PER_BAND: usize = 64;
/// 并行转换最多使用的线程数（再多会受内存带宽限制）。
const MAX_BANDS: usize = 4;

/// 把纹理映射出来的像素（`height` 行，每行起点相隔 `row_pitch` 字节）转换成 RGBA8，
/// alpha 固定为 255。大图按行分段并行转换（4K HDR 单线程约 20 ms）。
pub(crate) fn to_rgba(
    format: &DesktopFormat,
    width: usize,
    height: usize,
    row_pitch: usize,
    data: &[u8],
) -> Vec<u8> {
    let mut rgba = vec![0; width * height * 4];
    if rgba.is_empty() {
        return rgba;
    }
    let lut = match format {
        DesktopFormat::Bgra8 => None,
        DesktopFormat::ScRgbF16 { sdr_white } => Some(scrgb_to_srgb_lut(*sdr_white)),
    };
    let convert_row = |y: usize, out: &mut [u8]| {
        let row = &data[y * row_pitch..];
        match &lut {
            None => {
                for (p, o) in row[..width * 4]
                    .chunks_exact(4)
                    .zip(out.chunks_exact_mut(4))
                {
                    o.copy_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
            Some(lut) => {
                let channel = |lo: u8, hi: u8| lut[usize::from(u16::from_le_bytes([lo, hi]))];
                for (p, o) in row[..width * 8]
                    .chunks_exact(8)
                    .zip(out.chunks_exact_mut(4))
                {
                    o.copy_from_slice(&[
                        channel(p[0], p[1]),
                        channel(p[2], p[3]),
                        channel(p[4], p[5]),
                        255,
                    ]);
                }
            }
        }
    };
    let convert_band = |first_row: usize, band: &mut [u8]| {
        for (i, out) in band.chunks_exact_mut(width * 4).enumerate() {
            convert_row(first_row + i, out);
        }
    };

    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let bands = threads
        .min(MAX_BANDS)
        .min(height / MIN_ROWS_PER_BAND)
        .max(1);
    if bands == 1 {
        convert_band(0, &mut rgba);
        return rgba;
    }
    let rows_per_band = height.div_ceil(bands);
    std::thread::scope(|scope| {
        for (index, band) in rgba.chunks_mut(rows_per_band * width * 4).enumerate() {
            let convert_band = &convert_band;
            scope.spawn(move || convert_band(index * rows_per_band, band));
        }
    });
    rgba
}

/// 半精度浮点位模式 → sRGB 8 位分量的查找表：先按 SDR 白电平归一化，
/// 超出 SDR 范围的高光截断，再做 sRGB 编码。最近一次的表会缓存起来（白电平很少变）。
fn scrgb_to_srgb_lut(sdr_white: f32) -> Arc<[u8]> {
    static CACHE: Mutex<Option<(u32, Arc<[u8]>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    match &*cache {
        Some((bits, lut)) if *bits == sdr_white.to_bits() => return lut.clone(),
        _ => {}
    }
    let lut = build_scrgb_to_srgb_lut(sdr_white);
    *cache = Some((sdr_white.to_bits(), lut.clone()));
    lut
}

fn build_scrgb_to_srgb_lut(sdr_white: f32) -> Arc<[u8]> {
    (0..=u16::MAX)
        .map(|bits| {
            let linear = f16_to_f32(bits) / sdr_white;
            if linear.is_nan() || linear <= 0.0 {
                return 0;
            }
            let linear = linear.min(1.0);
            let encoded = if linear <= 0.003_130_8 {
                12.92 * linear
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (encoded * 255.0).round() as u8
        })
        .collect()
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    let magnitude = match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    };
    sign * magnitude
}

/// 目标图中 `(dx, dy)` 取自源图的哪个像素；`w`、`h` 是源图尺寸。
type SourcePixel = fn(dx: usize, dy: usize, w: usize, h: usize) -> (usize, usize);

/// Desktop Duplication 给出的是显示器原始朝向的画面；按显示器的旋转方向转回桌面朝向。
pub(crate) fn rotate(image: Image, rotation: DXGI_MODE_ROTATION) -> Image {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let (dw, dh, source): (usize, usize, SourcePixel) = match rotation {
        DXGI_MODE_ROTATION_ROTATE90 => (h, w, |dx, dy, _, h| (dy, h - 1 - dx)),
        DXGI_MODE_ROTATION_ROTATE180 => (w, h, |dx, dy, w, h| (w - 1 - dx, h - 1 - dy)),
        DXGI_MODE_ROTATION_ROTATE270 => (h, w, |dx, dy, w, _| (w - 1 - dy, dx)),
        _ => return image,
    };
    let src = image.rgba();
    let mut out = Vec::with_capacity(src.len());
    for dy in 0..dh {
        for dx in 0..dw {
            let (sx, sy) = source(dx, dy, w, h);
            let i = (sy * w + sx) * 4;
            out.extend_from_slice(&src[i..i + 4]);
        }
    }
    Image::from_rgba(dw as u32, dh as u32, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 半精度浮点的位模式。
    const F16_ONE: u16 = 0x3C00;
    const F16_HALF: u16 = 0x3800;
    const F16_TWO: u16 = 0x4000;
    const F16_MINUS_ONE: u16 = 0xBC00;

    fn f16_pixel(r: u16, g: u16, b: u16) -> Vec<u8> {
        [r, g, b, F16_ONE]
            .iter()
            .flat_map(|c| c.to_le_bytes())
            .collect()
    }

    #[test]
    fn bgra8_is_swizzled_to_opaque_rgba_ignoring_row_padding() {
        // 2×2，每行 8 字节像素 + 4 字节填充。
        let data = [
            1, 2, 3, 0, 4, 5, 6, 0, 0xEE, 0xEE, 0xEE, 0xEE, //
            7, 8, 9, 0, 10, 11, 12, 0,
        ];
        let rgba = to_rgba(&DesktopFormat::Bgra8, 2, 2, 12, &data);
        assert_eq!(
            rgba,
            [3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]
        );
    }

    #[test]
    fn scrgb_is_normalized_by_sdr_white_clipped_and_srgb_encoded() {
        let data: Vec<u8> = [
            f16_pixel(F16_ONE, F16_HALF, 0),
            f16_pixel(F16_TWO, F16_MINUS_ONE, F16_ONE),
        ]
        .concat();
        let format = DesktopFormat::ScRgbF16 { sdr_white: 1.0 };
        assert_eq!(
            to_rgba(&format, 2, 1, 16, &data),
            [255, 188, 0, 255, 255, 0, 255, 255]
        );
        let brighter_white = DesktopFormat::ScRgbF16 { sdr_white: 2.0 };
        assert_eq!(
            to_rgba(&brighter_white, 2, 1, 16, &data),
            [188, 137, 0, 255, 255, 0, 188, 255]
        );
    }

    #[test]
    fn large_images_keep_every_row_in_place() {
        let (width, height) = (300, 517);
        let row_pitch = width * 4 + 8;
        let mut data = vec![0u8; row_pitch * height];
        for y in 0..height {
            for x in 0..width {
                let p = y * row_pitch + x * 4;
                data[p + 2] = (y % 256) as u8; // R
                data[p] = (x % 256) as u8; // B
            }
        }
        let rgba = to_rgba(&DesktopFormat::Bgra8, width, height, row_pitch, &data);
        assert_eq!(rgba.len(), width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let p = (y * width + x) * 4;
                assert_eq!(
                    rgba[p..p + 4],
                    [(y % 256) as u8, 0, (x % 256) as u8, 255],
                    "({x}, {y})"
                );
            }
        }
    }
}
