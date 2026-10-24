//! 程序绘制的托盘图标：蓝色圆角方块上的白色取景框四角。

use bps_platform::Image;

const SIZE: u32 = 32;
const BACKGROUND: [u8; 4] = [0x25, 0x63, 0xEB, 0xFF];
const FOREGROUND: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// 32x32 的托盘图标，系统会按 DPI 缩放。
pub fn tray_icon() -> Image {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let color = if !in_rounded_square(x, y) {
                TRANSPARENT
            } else if on_viewfinder_corner(x, y) {
                FOREGROUND
            } else {
                BACKGROUND
            };
            pixels.extend_from_slice(&color);
        }
    }
    Image::from_rgba(SIZE, SIZE, pixels)
}

/// 外边距 1、圆角半径 7 的方块。
fn in_rounded_square(x: u32, y: u32) -> bool {
    let (min, max, radius) = (1.0, 31.0, 7.0);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    if px < min || py < min || px > max || py > max {
        return false;
    }
    let cx = px.clamp(min + radius, max - radius);
    let cy = py.clamp(min + radius, max - radius);
    (px - cx).powi(2) + (py - cy).powi(2) <= radius * radius
}

/// 取景框：范围 8..24，边长 6、粗 2 的四个直角。
fn on_viewfinder_corner(x: u32, y: u32) -> bool {
    let (low, high, length, thickness) = (8, 24, 6, 2);
    if !(low..high).contains(&x) || !(low..high).contains(&y) {
        return false;
    }
    let near_x_edge = x < low + thickness || x >= high - thickness;
    let near_y_edge = y < low + thickness || y >= high - thickness;
    let in_x_arm = x < low + length || x >= high - length;
    let in_y_arm = y < low + length || y >= high - length;
    (near_x_edge && in_y_arm) || (near_y_edge && in_x_arm)
}
