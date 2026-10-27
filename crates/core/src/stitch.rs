//! 跨显示器拼接：各显示器按自己的物理像素截取，再按它们在虚拟桌面中的位置拼成一张截图。
//!
//! 坐标一律是虚拟桌面物理像素（Per-Monitor V2 DPI 感知下，显示器的位置和截图都已是物理像素），
//! 所以不同缩放比例的显示器只需按位置摆放，不做任何缩放。没有任何显示器覆盖的空隙是透明像素。

use bps_platform::{Display, FrameSource, Image, Point, Rect};

use crate::CoreError;

/// 放在虚拟桌面里的一张图像：左上角位于 `origin`，一个像素对应一个物理像素。
///
/// 通常是一块显示器的整幅画面（`origin` 为显示器 `bounds` 的左上角），
/// 也可以是已经拼好的一大张图（例如整个虚拟桌面的冻结帧）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopImage {
    pub origin: Point,
    pub image: Image,
}

/// 虚拟桌面的范围：所有显示器的外接矩形。没有显示器时返回 `None`。
pub fn virtual_desktop_bounds(displays: &[Display]) -> Option<Rect> {
    displays
        .iter()
        .map(|display| display.bounds)
        .reduce(|bounds, next| bounds.union(next))
}

/// 截取虚拟桌面中 `rect` 范围内的画面：与 `rect` 重叠的每块显示器各截一张，再拼接、裁剪。
///
/// 输出尺寸恒等于 `rect` 的尺寸；`rect` 中没有任何显示器覆盖的部分是透明像素。
/// 虚拟桌面截图就是 `rect` 等于 [`virtual_desktop_bounds`] 的特例。
pub fn capture_rect(frame_source: &dyn FrameSource, rect: Rect) -> Result<Image, CoreError> {
    let displays = frame_source.displays()?;
    capture_displays(frame_source, &displays, rect)
}

/// 把已截下的图像按各自在虚拟桌面中的位置拼接，并裁剪成 `rect` 范围。
///
/// 输出尺寸恒等于 `rect` 的尺寸；没有任何图像覆盖的部分是透明像素；
/// 图像互相重叠时，`parts` 中靠后的覆盖靠前的。
pub fn stitch(rect: Rect, parts: &[DesktopImage]) -> Image {
    let mut canvas = Canvas::new(rect);
    for part in parts {
        canvas.paint(part.origin, &part.image);
    }
    canvas.finish()
}

/// 同 [`capture_rect`]，显示器布局由调用方提供（避免重复枚举显示器）。
pub(crate) fn capture_displays(
    frame_source: &dyn FrameSource,
    displays: &[Display],
    rect: Rect,
) -> Result<Image, CoreError> {
    let mut canvas = Canvas::new(rect);
    for display in displays {
        if display.bounds.intersection(rect).is_none() {
            continue;
        }
        // 截一块、画一块，同一时刻只保留一块显示器的整幅画面。
        let image = frame_source.capture_display(display.id)?;
        canvas.paint(Point::new(display.bounds.x, display.bounds.y), &image);
    }
    Ok(canvas.finish())
}

/// 拼接中的截图：覆盖 `rect`，初始全透明。
struct Canvas {
    rect: Rect,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(rect: Rect) -> Self {
        Self {
            rect,
            pixels: vec![0; rect.width as usize * rect.height as usize * 4],
        }
    }

    /// 把左上角位于 `origin` 的图像画上去，只画与画布重叠的部分；后画的覆盖先画的。
    fn paint(&mut self, origin: Point, image: &Image) {
        let bounds = Rect::new(origin.x, origin.y, image.width(), image.height());
        let Some(overlap) = bounds.intersection(self.rect) else {
            return;
        };
        let offset = |from: i32, to: i32| (i64::from(to) - i64::from(from)) as usize;
        let (source_x, source_y) = (offset(origin.x, overlap.x), offset(origin.y, overlap.y));
        let (target_x, target_y) = (
            offset(self.rect.x, overlap.x),
            offset(self.rect.y, overlap.y),
        );
        let row_bytes = overlap.width as usize * 4;
        let source_stride = image.width() as usize * 4;
        let target_stride = self.rect.width as usize * 4;
        for row in 0..overlap.height as usize {
            let source = (source_y + row) * source_stride + source_x * 4;
            let target = (target_y + row) * target_stride + target_x * 4;
            self.pixels[target..target + row_bytes]
                .copy_from_slice(&image.rgba()[source..source + row_bytes]);
        }
    }

    fn finish(self) -> Image {
        Image::from_rgba(self.rect.width, self.rect.height, self.pixels)
    }
}
