//! 长截屏期间区域旁边的拼接预览，以及完成后长截屏在标注时的缩小视图。
//!
//! 预览按固定比例缩小显示拼接结果中最近一帧及其上方的内容（预览框放不下全部时只显示到最近
//! 一帧的底边为止的那一段），用户滚动时能看清刚拼上的地方有没有重复或缺行。

use bps_platform::{Display, Image, Rect};

use super::LongScreenshot;
use crate::downscale::downscale_rows;

/// 以下尺寸是 100% 缩放下的值（物理像素），按区域所在显示器的缩放比例放大。
/// 预览框与区域边框之间的距离。
const GAP: f64 = 8.0;
/// 预览框的宽度占区域宽度的比例，以及宽度的上下限。
const WIDTH_RATIO: f64 = 0.35;
const MIN_WIDTH: f64 = 80.0;
const MAX_WIDTH: f64 = 320.0;
/// 预览框至少这么高（区域很矮时）。
const MIN_HEIGHT: f64 = 160.0;
/// 预览框、标注视图离显示器边缘至少这么远。
const MARGIN: f64 = 12.0;
/// 标注视图下方给标注工具条留出的高度（工具条高 38，再加间距）。
const TOOLBAR_SPACE: f64 = 50.0;

/// 区域所在的显示器（与区域重叠最多的那块）。
pub(crate) fn display_of(region: Rect, displays: &[Display]) -> Option<&Display> {
    displays.iter().max_by_key(|d| {
        d.bounds
            .intersection(region)
            .map_or(0, |r| u64::from(r.width) * u64::from(r.height))
    })
}

/// 预览框：区域右边（放不下就左边，都放不下就区域内的右侧）、与区域顶边对齐的一块范围。
/// 返回预览框和区域所在显示器的缩放比例。
pub(crate) fn preview_box(region: Rect, frame: u32, displays: &[Display]) -> (Rect, f64) {
    let (screen, scale) =
        display_of(region, displays).map_or((region, 1.0), |d| (d.bounds, d.scale_factor));
    let px = |v: f64| (v * scale).round() as i32;
    let width = (f64::from(region.width) * WIDTH_RATIO)
        .clamp(MIN_WIDTH * scale, MAX_WIDTH * scale)
        .min(f64::from(region.width))
        .max(1.0) as i32;
    let margin = px(MARGIN);
    let screen_top = screen.y + margin;
    let screen_bottom = screen.y + screen.height as i32 - margin;
    let height = (region.height as i32)
        .max(px(MIN_HEIGHT))
        .min(screen_bottom - screen_top)
        .max(1);
    let top = region.y.min(screen_bottom - height).max(screen_top);
    let gap = px(GAP) + frame as i32;
    let region_right = region.x + region.width as i32;
    let left = if region_right + gap + width <= screen.x + screen.width as i32 - margin {
        region_right + gap
    } else if region.x - gap - width >= screen.x + margin {
        region.x - gap - width
    } else {
        // 两边都放不下：放在区域内的右侧（遮罩不出现在采样的画面里，不影响拼接）。
        (region_right - gap - width).max(region.x)
    };
    (Rect::new(left, top, width as u32, height as u32), scale)
}

/// 拼接预览的画面和它在屏幕上的位置（预览框左上角对齐）：按预览框宽度等比缩小，显示到
/// 最近一帧底边为止、预览框高度容得下的那一段。
pub(crate) fn render(long: &LongScreenshot, frame_box: Rect) -> (Rect, Image) {
    let (width, height) = (long.width().max(1), long.height().max(1));
    let scale = (f64::from(frame_box.width) / f64::from(width)).min(1.0);
    let visible = ((f64::from(frame_box.height) / scale).floor() as u32).clamp(1, height);
    let frame_bottom = long.frame_span().1.clamp(0, i64::from(height)) as u32;
    let bottom = frame_bottom.max(visible);
    let top = bottom - visible;
    let out_width = ((f64::from(width) * scale).round() as u32).clamp(1, frame_box.width.max(1));
    let out_height =
        ((f64::from(visible) * scale).round() as u32).clamp(1, frame_box.height.max(1));
    let image = downscale_rows(long.rgba(), width, top, bottom, out_width, out_height);
    (
        Rect::new(frame_box.x, frame_box.y, out_width, out_height),
        image,
    )
}

/// 长截屏在标注时的视图：整张长截屏等比缩小（不放大）到区域所在显示器放得下的大小，
/// 下方留出标注工具条的位置。尽量与区域左边、顶边对齐。
pub(crate) fn annotation_view(region: Rect, image: &Image, displays: &[Display]) -> Rect {
    let (screen, scale) =
        display_of(region, displays).map_or((region, 1.0), |d| (d.bounds, d.scale_factor));
    let px = |v: f64| (v * scale).round() as i32;
    let margin = px(MARGIN);
    let available_width = (screen.width as i32 - 2 * margin).max(1);
    let available_height = (screen.height as i32 - 2 * margin - px(TOOLBAR_SPACE)).max(1);
    let (width, height) = (image.width().max(1), image.height().max(1));
    let fit = (f64::from(available_width) / f64::from(width))
        .min(f64::from(available_height) / f64::from(height))
        .min(1.0);
    let view_width = ((f64::from(width) * fit).round() as i32).clamp(1, available_width);
    let view_height = ((f64::from(height) * fit).round() as i32).clamp(1, available_height);
    let (low_x, low_y) = (screen.x + margin, screen.y + margin);
    let left = region
        .x
        .clamp(low_x, (low_x + available_width - view_width).max(low_x));
    let top = region
        .y
        .clamp(low_y, (low_y + available_height - view_height).max(low_y));
    Rect::new(left, top, view_width as u32, view_height as u32)
}
