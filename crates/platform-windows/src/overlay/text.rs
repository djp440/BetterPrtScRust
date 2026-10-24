//! 标注文字的栅格化：DirectWrite 排版，Direct2D 以灰度抗锯齿画进一张 A8 位图，再读回 CPU
//! 作为覆盖率蒙版（ADR 0006）。用遮罩常驻的 Direct2D 设备，不另建设备。

use bps_platform::{PlatformError, Rect, TextMask};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_MAP_OPTIONS_READ, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1Bitmap1,
};
use windows::Win32::Graphics::DirectWrite::{DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_METRICS};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_A8_UNORM;
use windows_numerics::Vector2;

use super::gpu::Gpu;
use crate::error::win_error;

/// 排版的最大宽高：不自动换行，只要足够大。
const MAX_EXTENT: f32 = 16384.0;

/// 把 `text` 按字号 `font_size`（物理像素）排版并栅格化成覆盖率蒙版。
pub(crate) fn rasterize(gpu: &Gpu, text: &str, font_size: u32) -> Result<TextMask, PlatformError> {
    let format = gpu.annotation_text_format(font_size.max(1) as f32)?;
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let layout = gpu
            .dwrite()
            .CreateTextLayout(&wide, &format, MAX_EXTENT, MAX_EXTENT)
            .map_err(|e| win_error("排版标注文字失败", e))?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout
            .GetMetrics(&mut metrics)
            .map_err(|e| win_error("排版标注文字失败", e))?;
        let width = metrics.widthIncludingTrailingWhitespace.max(0.0);
        let height = metrics.height.max(1.0);
        let layout_box = Rect::new(0, 0, width.ceil() as u32, height.ceil() as u32);

        // 插入点：文字末尾（最后一个换行之后则是新一行的开头）。
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        layout
            .HitTestTextPosition(wide.len() as u32, false, &mut x, &mut y, &mut hit)
            .map_err(|e| win_error("定位插入点失败", e))?;
        let line_height = if hit.height > 0.0 { hit.height } else { height };
        let caret = Rect::new(
            x.round() as i32,
            y.round() as i32,
            (font_size / 16).max(1),
            line_height.ceil() as u32,
        );

        if text.chars().all(char::is_whitespace) {
            return Ok(TextMask::new(
                Rect::default(),
                Vec::new(),
                layout_box,
                caret,
            ));
        }

        // 字形可能伸出排版框（例如 j、f 的左右伸展）：按伸出量扩大，再各留 1 像素给抗锯齿。
        layout
            .SetMaxWidth(width)
            .and_then(|()| layout.SetMaxHeight(height))
            .map_err(|e| win_error("排版标注文字失败", e))?;
        let overhang = layout
            .GetOverhangMetrics()
            .map_err(|e| win_error("排版标注文字失败", e))?;
        let out = |v: f32| v.max(0.0).ceil() as i32 + 1;
        let left = -out(overhang.left);
        let top = -out(overhang.top);
        let right = width.ceil() as i32 + out(overhang.right);
        let bottom = height.ceil() as i32 + out(overhang.bottom);
        let area = Rect::new(left, top, (right - left) as u32, (bottom - top) as u32);
        let size = D2D_SIZE_U {
            width: area.width,
            height: area.height,
        };

        let ctx = &gpu.d2d;
        let target = ctx
            .CreateBitmap(
                size,
                None,
                0,
                &alpha_bitmap(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
            )
            .map_err(|e| win_error("创建文字位图失败", e))?;
        ctx.SetTarget(&target);
        ctx.BeginDraw();
        ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        ctx.Clear(Some(&D2D1_COLOR_F::default()));
        gpu.brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        });
        ctx.DrawTextLayout(
            Vector2 {
                X: -left as f32,
                Y: -top as f32,
            },
            &layout,
            &gpu.brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
        let drawn = ctx
            .EndDraw(None, None)
            .map_err(|e| win_error("栅格化标注文字失败", e));
        ctx.SetTarget(None);
        drawn?;

        let coverage = read_back(gpu, &target, size)?;
        Ok(TextMask::new(area, coverage, layout_box, caret))
    }
}

/// 把 A8 位图读回 CPU，按行排列、行间无填充。
unsafe fn read_back(
    gpu: &Gpu,
    target: &ID2D1Bitmap1,
    size: D2D_SIZE_U,
) -> Result<Vec<u8>, PlatformError> {
    unsafe {
        let readable = gpu
            .d2d
            .CreateBitmap(
                size,
                None,
                0,
                &alpha_bitmap(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
            )
            .map_err(|e| win_error("创建文字读回位图失败", e))?;
        readable
            .CopyFromBitmap(None, target, None)
            .map_err(|e| win_error("读回标注文字失败", e))?;
        let mapped = readable
            .Map(D2D1_MAP_OPTIONS_READ)
            .map_err(|e| win_error("读回标注文字失败", e))?;
        let (width, height) = (size.width as usize, size.height as usize);
        let mut coverage = Vec::with_capacity(width * height);
        for row in 0..height {
            let line =
                std::slice::from_raw_parts(mapped.bits.add(row * mapped.pitch as usize), width);
            coverage.extend_from_slice(line);
        }
        let _ = readable.Unmap();
        Ok(coverage)
    }
}

/// 只有 alpha 通道的位图属性（覆盖率蒙版）。
fn alpha_bitmap(options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        ..Default::default()
    }
}
