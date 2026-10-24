//! 标注样式：工具条上的 5 种预设颜色和 3 档线宽。
//!
//! “线宽”对不同工具的含义不同：框和箭头是线条宽度，文字是字号，马赛克是色块边长。
//! 下面的尺寸都是 100% 缩放下的物理像素，按开始画的那块显示器的缩放比例放大。

use bps_platform::Color;

/// 工具条上的预设颜色，从左到右；第一种是默认颜色。
pub(crate) const PALETTE: [Color; 5] = [
    Color::rgba(240, 40, 40, 255),
    Color::rgba(255, 190, 0, 255),
    Color::rgba(40, 170, 70, 255),
    Color::rgba(30, 110, 240, 255),
    Color::rgba(0, 0, 0, 255),
];

/// 三档线宽。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Thickness {
    Thin,
    #[default]
    Medium,
    Thick,
}

impl Thickness {
    /// 工具条上从左到右的顺序。
    pub const ALL: [Thickness; 3] = [Thickness::Thin, Thickness::Medium, Thickness::Thick];

    /// 框和箭头的线宽。
    fn stroke_width(self) -> f64 {
        match self {
            Thickness::Thin => 2.0,
            Thickness::Medium => 4.0,
            Thickness::Thick => 6.0,
        }
    }

    /// 文字的字号。
    fn font_size(self) -> f64 {
        match self {
            Thickness::Thin => 16.0,
            Thickness::Medium => 22.0,
            Thickness::Thick => 30.0,
        }
    }

    /// 马赛克的色块边长。
    fn mosaic_block(self) -> f64 {
        match self {
            Thickness::Thin => 10.0,
            Thickness::Medium => 16.0,
            Thickness::Thick => 24.0,
        }
    }

    /// 缩放比例为 `scale` 的显示器上框和箭头的线宽（物理像素，至少 1）。
    pub fn stroke_width_at(self, scale: f64) -> u32 {
        scaled(self.stroke_width(), scale)
    }

    /// 缩放比例为 `scale` 的显示器上文字的字号（物理像素）。
    pub fn font_size_at(self, scale: f64) -> u32 {
        scaled(self.font_size(), scale)
    }

    /// 缩放比例为 `scale` 的显示器上马赛克的色块边长（物理像素）。
    pub fn mosaic_block_at(self, scale: f64) -> u32 {
        scaled(self.mosaic_block(), scale)
    }

    /// 工具条上表示这一档的线条样本的粗细（100% 缩放下）。
    pub fn sample_height(self) -> f64 {
        self.stroke_width()
    }
}

fn scaled(value: f64, scale: f64) -> u32 {
    ((value * scale).round() as u32).max(1)
}
