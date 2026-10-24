//! 把 Desktop Duplication 的像素格式转换成 `bps_platform::Image` 的 RGBA8。

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

/// 逐行把 `rows`（每行是纹理中的一行原始字节）转换成 RGBA8，alpha 固定为 255。
pub(crate) fn to_rgba<'a>(
    format: &DesktopFormat,
    width: usize,
    rows: impl Iterator<Item = &'a [u8]>,
) -> Vec<u8> {
    let mut rgba = Vec::new();
    match format {
        DesktopFormat::Bgra8 => {
            for row in rows {
                for p in row[..width * 4].chunks_exact(4) {
                    rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
        }
        DesktopFormat::ScRgbF16 { sdr_white } => {
            let lut = scrgb_to_srgb_lut(*sdr_white);
            let channel = |lo: u8, hi: u8| lut[usize::from(u16::from_le_bytes([lo, hi]))];
            for row in rows {
                for p in row[..width * 8].chunks_exact(8) {
                    rgba.extend_from_slice(&[
                        channel(p[0], p[1]),
                        channel(p[2], p[3]),
                        channel(p[4], p[5]),
                        255,
                    ]);
                }
            }
        }
    }
    rgba
}

/// 半精度浮点位模式 → sRGB 8 位分量的查找表：先按 SDR 白电平归一化，
/// 超出 SDR 范围的高光截断，再做 sRGB 编码。
fn scrgb_to_srgb_lut(sdr_white: f32) -> Vec<u8> {
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
