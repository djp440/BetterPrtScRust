//! 把 [`OverlayScene`] 画到遮罩窗口上：先画冻结帧，再画画布（有的话），最后按顺序画场景里的元素。
//!
//! 场景坐标是虚拟桌面物理像素，减去窗口左上角 `origin` 就是窗口内的像素坐标。
//! 新的 [`OverlayItem`] 变体在 [`draw_item`] 里加一个分支。

use bps_platform::{Color, Corner, Label, OverlayItem, OverlayScene, PlatformError, Point, Rect};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1Bitmap1,
};
use windows::Win32::Graphics::DirectWrite::{DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS};

use super::gpu::{FrozenDisplay, Gpu};
use crate::error::win_error;

/// 画布：盖在冻结帧上、由 `core` 提供像素的位图。
pub(crate) struct Canvas {
    /// 在虚拟桌面中的范围。
    pub area: Rect,
    pub bitmap: ID2D1Bitmap1,
}

/// 在 `target` 上画一整帧（不含呈现）。`origin` 是窗口左上角在虚拟桌面中的位置。
pub(crate) fn draw(
    gpu: &Gpu,
    target: &ID2D1Bitmap1,
    origin: Point,
    frozen: &[FrozenDisplay],
    canvas: Option<&Canvas>,
    scene: &OverlayScene,
) -> Result<(), PlatformError> {
    let ctx = &gpu.d2d;
    unsafe {
        ctx.SetTarget(target);
        ctx.BeginDraw();
        // 场景里的矩形都在整数像素上，关掉抗锯齿才能像素对齐。
        ctx.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        ctx.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }));
        for display in frozen {
            ctx.DrawBitmap(
                &display.bitmap,
                Some(&rect_f(display.display.bounds, origin)),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        if let Some(canvas) = canvas {
            ctx.DrawBitmap(
                &canvas.bitmap,
                Some(&rect_f(canvas.area, origin)),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        let drawn = scene
            .items
            .iter()
            .try_for_each(|item| draw_item(gpu, origin, item));
        let ended = ctx
            .EndDraw(None, None)
            .map_err(|e| win_error("绘制遮罩失败", e));
        ctx.SetTarget(None);
        drawn.and(ended)
    }
}

fn draw_item(gpu: &Gpu, origin: Point, item: &OverlayItem) -> Result<(), PlatformError> {
    match item {
        OverlayItem::Fill { rect, color } => {
            fill(gpu, rect_f(*rect, origin), *color);
            Ok(())
        }
        OverlayItem::Frame { rect, width, color } => {
            let r = rect_f(*rect, origin);
            let w = *width as f32;
            for edge in [
                D2D_RECT_F {
                    left: r.left - w,
                    top: r.top - w,
                    right: r.right + w,
                    bottom: r.top,
                },
                D2D_RECT_F {
                    left: r.left - w,
                    top: r.bottom,
                    right: r.right + w,
                    bottom: r.bottom + w,
                },
                D2D_RECT_F {
                    left: r.left - w,
                    top: r.top,
                    right: r.left,
                    bottom: r.bottom,
                },
                D2D_RECT_F {
                    left: r.right,
                    top: r.top,
                    right: r.right + w,
                    bottom: r.bottom,
                },
            ] {
                fill(gpu, edge, *color);
            }
            Ok(())
        }
        OverlayItem::Label(label) => draw_label(gpu, origin, label),
        OverlayItem::Text {
            rect,
            text,
            font_size,
            color,
        } => draw_centered_text(gpu, rect_f(*rect, origin), text, *font_size, *color),
    }
}

/// 在 `rect` 内居中画一行文字。
fn draw_centered_text(
    gpu: &Gpu,
    rect: D2D_RECT_F,
    text: &str,
    font_size: f32,
    color: Color,
) -> Result<(), PlatformError> {
    let format = gpu.centered_text_format(font_size)?;
    let text: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        gpu.brush.SetColor(&color_f(color));
        gpu.d2d.DrawText(
            &text,
            &format,
            &rect,
            &gpu.brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

fn draw_label(gpu: &Gpu, origin: Point, label: &Label) -> Result<(), PlatformError> {
    let format = gpu.text_format(label.font_size)?;
    let text: Vec<u16> = label.text.encode_utf16().collect();
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe {
        gpu.dwrite()
            .CreateTextLayout(&text, &format, 16384.0, 16384.0)
            .and_then(|layout| layout.GetMetrics(&mut metrics))
            .map_err(|e| win_error("排版标签文字失败", e))?;
    }
    let padding = label.padding as f32;
    let width = metrics.widthIncludingTrailingWhitespace.ceil() + 2.0 * padding;
    let height = metrics.height.ceil() + 2.0 * padding;
    let left = (label.anchor.x - origin.x) as f32;
    let top = match label.corner {
        Corner::TopLeft => (label.anchor.y - origin.y) as f32,
        Corner::BottomLeft => (label.anchor.y - origin.y) as f32 - height,
    };
    let background = D2D_RECT_F {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    fill(gpu, background, label.background);
    unsafe {
        gpu.brush.SetColor(&color_f(label.color));
        gpu.d2d.DrawText(
            &text,
            &format,
            &D2D_RECT_F {
                left: left + padding,
                top: top + padding,
                right: background.right,
                bottom: background.bottom,
            },
            &gpu.brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

fn fill(gpu: &Gpu, rect: D2D_RECT_F, color: Color) {
    unsafe {
        gpu.brush.SetColor(&color_f(color));
        gpu.d2d.FillRectangle(&rect, &gpu.brush);
    }
}

fn rect_f(rect: Rect, origin: Point) -> D2D_RECT_F {
    let left = (i64::from(rect.x) - i64::from(origin.x)) as f32;
    let top = (i64::from(rect.y) - i64::from(origin.y)) as f32;
    D2D_RECT_F {
        left,
        top,
        right: left + rect.width as f32,
        bottom: top + rect.height as f32,
    }
}

fn color_f(color: Color) -> D2D1_COLOR_F {
    let channel = |v: u8| f32::from(v) / 255.0;
    D2D1_COLOR_F {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        a: channel(color.a),
    }
}
