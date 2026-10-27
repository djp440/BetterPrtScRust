//! 把图像缩小显示：长截屏的拼接预览、比屏幕还高的长截屏在标注时的缩小视图。

use bps_platform::Image;

/// 把 `pixels`（宽 `width` 的 RGBA 行）中第 `top` 行（含）到第 `bottom` 行（不含）这一段
/// 缩小成 `out_width` × `out_height`：每个输出像素取它覆盖的源像素的平均值。
///
/// 输出尺寸不应大于源尺寸（只缩小、不放大）；为 0 的尺寸按 1 处理。
pub(crate) fn downscale_rows(
    pixels: &[u8],
    width: u32,
    top: u32,
    bottom: u32,
    out_width: u32,
    out_height: u32,
) -> Image {
    let (out_width, out_height) = (out_width.max(1), out_height.max(1));
    let rows = bottom.saturating_sub(top).max(1);
    let width = width.max(1);
    let row_bytes = width as usize * 4;
    let mut out = Vec::with_capacity(out_width as usize * out_height as usize * 4);
    for oy in 0..out_height {
        let y0 = top + (u64::from(oy) * u64::from(rows) / u64::from(out_height)) as u32;
        let y1 = (top + (u64::from(oy + 1) * u64::from(rows) / u64::from(out_height)) as u32)
            .max(y0 + 1);
        for ox in 0..out_width {
            let x0 = (u64::from(ox) * u64::from(width) / u64::from(out_width)) as u32;
            let x1 =
                ((u64::from(ox + 1) * u64::from(width) / u64::from(out_width)) as u32).max(x0 + 1);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                let line = &pixels[y as usize * row_bytes..(y as usize + 1) * row_bytes];
                for pixel in line[x0 as usize * 4..x1 as usize * 4].chunks_exact(4) {
                    for (s, &p) in sum.iter_mut().zip(pixel) {
                        *s += u32::from(p);
                    }
                }
            }
            let count = (y1 - y0) * (x1 - x0);
            out.extend(sum.iter().map(|&s| ((s + count / 2) / count) as u8));
        }
    }
    Image::from_rgba(out_width, out_height, out)
}

/// 把整张 `image` 缩小成 `width` × `height`。
pub(crate) fn downscale(image: &Image, width: u32, height: u32) -> Image {
    downscale_rows(
        image.rgba(),
        image.width(),
        0,
        image.height(),
        width,
        height,
    )
}
