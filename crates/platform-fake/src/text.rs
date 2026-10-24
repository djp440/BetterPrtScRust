use bps_platform::{Rect, TextMask};

/// 假平台的文字栅格化（[`SelectionOverlay::rasterize_text`](bps_platform::SelectionOverlay::rasterize_text)
/// 的假实现），规则简单、可以直接写出期望值：
///
/// - 行高 `round(1.25 × 字号)`；遇到 `\n` 换行。
/// - 每个字符占一格：ASCII 字符宽 `round(0.6 × 字号)`，其余（汉字等）宽一个字号。
/// - 非空白字符画成一个完全覆盖（255）的实心方块：字符格左右各缩进 1 像素，上下各缩进
///   `字号 / 5`（取整）。
/// - 蒙版和排版框都从原点开始；插入点在最后一行末尾，宽 `max(1, 字号 / 16)`、高一行。
pub fn text_mask(text: &str, font_size: u32) -> TextMask {
    let size = font_size.max(1);
    let line_height = ((f64::from(size) * 1.25).round() as u32).max(1);
    let advance = |c: char| {
        if c.is_ascii() {
            (f64::from(size) * 0.6).round() as u32
        } else {
            size
        }
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let line_width = |line: &str| line.chars().map(advance).sum::<u32>();
    let width = lines.iter().map(|l| line_width(l)).max().unwrap_or(0);
    let height = line_height * lines.len() as u32;
    let mut coverage = vec![0u8; width as usize * height as usize];
    let inset = size / 5;
    for (row, line) in lines.iter().enumerate() {
        let top = row as u32 * line_height;
        let mut pen = 0;
        for c in line.chars() {
            let cell = advance(c);
            if !c.is_whitespace() {
                for y in top + inset..top + line_height - inset {
                    for x in pen + 1..(pen + cell).saturating_sub(1) {
                        coverage[(y * width + x) as usize] = 255;
                    }
                }
            }
            pen += cell;
        }
    }
    let last = lines.last().map_or(0, |l| line_width(l));
    TextMask::new(
        Rect::new(0, 0, width, height),
        coverage,
        Rect::new(0, 0, width, height),
        Rect::new(
            last as i32,
            (height - line_height) as i32,
            (size / 16).max(1),
            line_height,
        ),
    )
}
