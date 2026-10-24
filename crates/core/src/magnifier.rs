//! 放大镜：框选时贴在光标旁边，显示对准的像素四周放大的像素网格和这个像素的颜色值；
//! 按 C 把颜色值以 `#RRGGBB` 文本复制到剪贴板。
//!
//! 什么时候显示、对准哪个像素由选区状态机决定（`RegionSelection::magnifier_focus`），这里只管
//! 读像素和画。像素读自冻结帧，每次只读对准的像素四周一小块
//! （[`SelectionOverlay::frozen_pixels`]），指针没动时不再读。画面全部由现有的遮罩元素组合：
//! 每个像素一个填充格子、十字准线、网格线、对准像素的双色边框，下方是颜色值和操作提示。

use bps_platform::{
    ClipboardItem, Color, Display, Image, OverlayItem, Platform, PlatformError, Point, Rect,
    SelectionOverlay,
};

use crate::Texts;
use crate::stitch::{DesktopImage, stitch, virtual_desktop_bounds};

/// 对准的像素四周各显示几个像素：一共显示 `SPAN × SPAN` 个像素。
const RADIUS: i32 = 7;
const SPAN: i32 = 2 * RADIUS + 1;
/// 以下尺寸是 100% 缩放下的值（物理像素），按对准的像素所在显示器的缩放比例放大。
/// 一个像素放大后的边长。
const CELL: f64 = 8.0;
/// 放大镜与光标之间的距离。
const OFFSET: f64 = 20.0;
/// 下方颜色值、提示每行的高度。
const ROW_HEIGHT: f64 = 22.0;
const FONT_SIZE: f64 = 12.0;
/// 颜色值左边的色块。
const SWATCH: f64 = 12.0;
const PADDING: f64 = 6.0;

const BACKGROUND: Color = Color::rgba(32, 32, 32, 240);
const BORDER: Color = Color::rgba(255, 255, 255, 200);
const GRID: Color = Color::rgba(128, 128, 128, 80);
/// 十字准线：对准的像素所在的行和列，与选区边框同色。
const CROSSHAIR: Color = Color::rgba(30, 144, 255, 70);
const DARK: Color = Color::rgba(0, 0, 0, 255);
const LIGHT: Color = Color::rgba(255, 255, 255, 255);
const TEXT: Color = Color::rgba(255, 255, 255, 255);
const HINT: Color = Color::rgba(200, 200, 200, 255);

/// 一次捕获里的放大镜。
pub(crate) struct Magnifier {
    displays: Vec<Display>,
    /// 提示文字的语言。
    texts: &'static Texts,
    /// 最近一次读取的像素：对准的像素和它四周 `SPAN × SPAN` 的画面（没有显示器的地方透明）。
    sample: Option<(Point, Image)>,
    /// 最近一次按 C：对准的像素和是否复制成功。对准别的像素后不再提示。
    copied: Option<(Point, bool)>,
}

impl Magnifier {
    /// 在冻结帧覆盖的这些显示器上放大，提示文字用 `texts` 的语言。
    pub fn new(displays: Vec<Display>, texts: &'static Texts) -> Self {
        Self {
            displays,
            texts,
            sample: None,
            copied: None,
        }
    }

    /// 对准 `focus` 时放大镜的绘制元素，从下往上。
    pub fn items(
        &mut self,
        overlay: &dyn SelectionOverlay,
        focus: Point,
    ) -> Result<Vec<OverlayItem>, PlatformError> {
        let display = self.displays.iter().find(|d| d.bounds.contains(focus));
        let scale = display.map_or(1.0, |d| d.scale_factor);
        let screen = display
            .map(|d| d.bounds)
            .or_else(|| virtual_desktop_bounds(&self.displays))
            .unwrap_or_default();
        let px = |v: f64| ((v * scale).round() as i32).max(1);
        let cell = px(CELL).max(2);
        let view = cell * SPAN;
        let row = px(ROW_HEIGHT);
        let (width, height) = (view, view + 2 * row);
        let (left, top) = place(focus, (width, height), px(OFFSET), screen);
        let rect = |x: i32, y: i32, w: i32, h: i32| Rect::new(x, y, w as u32, h as u32);

        let pixels = self.sample(overlay, focus)?;
        let color = center_color(pixels);
        let mut items = vec![OverlayItem::Fill {
            rect: rect(left, top, width, height),
            color: BACKGROUND,
        }];
        for y in 0..SPAN {
            for x in 0..SPAN {
                let [r, g, b, a] = pixels.pixel(x as u32, y as u32);
                if a == 0 {
                    continue;
                }
                items.push(OverlayItem::Fill {
                    rect: rect(left + x * cell, top + y * cell, cell, cell),
                    color: Color::rgba(r, g, b, 255),
                });
            }
        }
        let (center_x, center_y) = (left + RADIUS * cell, top + RADIUS * cell);
        let arm = RADIUS * cell;
        for band in [
            rect(left, center_y, arm, cell),
            rect(center_x + cell, center_y, arm, cell),
            rect(center_x, top, cell, arm),
            rect(center_x, center_y + cell, cell, arm),
        ] {
            items.push(OverlayItem::Fill {
                rect: band,
                color: CROSSHAIR,
            });
        }
        for k in 1..SPAN {
            items.push(OverlayItem::Fill {
                rect: rect(left + k * cell, top, 1, view),
                color: GRID,
            });
            items.push(OverlayItem::Fill {
                rect: rect(left, top + k * cell, view, 1),
                color: GRID,
            });
        }
        // 对准的像素：里黑外白两圈边框，深色、浅色画面上都看得清。
        let line = px(1.0);
        items.push(OverlayItem::Frame {
            rect: rect(center_x, center_y, cell, cell),
            width: line as u32,
            color: DARK,
        });
        items.push(OverlayItem::Frame {
            rect: rect(
                center_x - line,
                center_y - line,
                cell + 2 * line,
                cell + 2 * line,
            ),
            width: line as u32,
            color: LIGHT,
        });
        items.push(OverlayItem::Frame {
            rect: rect(left, top, width, height),
            width: line as u32,
            color: BORDER,
        });

        let font_size = (FONT_SIZE * scale) as f32;
        let padding = px(PADDING);
        let first_row = top + view;
        if let Some(color) = color {
            let swatch = px(SWATCH);
            let swatch_rect = rect(
                left + padding,
                first_row + (row - swatch) / 2,
                swatch,
                swatch,
            );
            items.push(OverlayItem::Fill {
                rect: swatch_rect,
                color,
            });
            items.push(OverlayItem::Frame {
                rect: swatch_rect,
                width: line as u32,
                color: BORDER,
            });
            let text_left = left + padding + swatch;
            items.push(OverlayItem::Text {
                rect: rect(
                    text_left,
                    first_row,
                    left + width - padding - text_left,
                    row,
                ),
                text: hex(color),
                font_size,
                color: TEXT,
            });
        }
        let hint = match self.copied {
            Some((at, true)) if at == focus => "overlay-color-copied",
            Some((at, false)) if at == focus => "overlay-color-copy-failed",
            _ => "overlay-copy-color-hint",
        };
        items.push(OverlayItem::Text {
            rect: rect(left, first_row + row, width, row),
            text: self.texts.get(hint),
            font_size,
            color: HINT,
        });
        Ok(items)
    }

    /// 把 `focus` 处像素的颜色以 `#RRGGBB` 文本复制到剪贴板；放大镜随后提示复制成功或失败。
    /// `focus` 不在任何显示器上时什么也不做。
    pub fn copy_color(
        &mut self,
        platform: &mut Platform,
        focus: Point,
    ) -> Result<(), PlatformError> {
        let Some(color) = center_color(self.sample(platform.overlay.as_ref(), focus)?) else {
            return Ok(());
        };
        let written = platform
            .clipboard
            .write(vec![ClipboardItem::Text(hex(color))]);
        self.copied = Some((focus, written.is_ok()));
        Ok(())
    }

    /// 对准 `focus` 时要放大的像素；指针没动时沿用上一次读到的。
    fn sample(
        &mut self,
        overlay: &dyn SelectionOverlay,
        focus: Point,
    ) -> Result<&Image, PlatformError> {
        if self.sample.as_ref().is_none_or(|(at, _)| *at != focus) {
            let area = Rect::new(focus.x - RADIUS, focus.y - RADIUS, SPAN as u32, SPAN as u32);
            let parts = self
                .displays
                .iter()
                .filter_map(|d| d.bounds.intersection(area).map(|overlap| (d, overlap)))
                .map(|(display, overlap)| {
                    let local = Rect::new(
                        overlap.x - display.bounds.x,
                        overlap.y - display.bounds.y,
                        overlap.width,
                        overlap.height,
                    );
                    Ok(DesktopImage {
                        origin: Point::new(overlap.x, overlap.y),
                        image: overlay.frozen_pixels(display.id, local)?,
                    })
                })
                .collect::<Result<Vec<_>, PlatformError>>()?;
            self.sample = Some((focus, stitch(area, &parts)));
        }
        Ok(&self.sample.as_ref().expect("刚刚读取过").1)
    }
}

/// 读到的像素中对准的那一个；它不在任何显示器上（透明）时为 `None`。
fn center_color(pixels: &Image) -> Option<Color> {
    let [r, g, b, a] = pixels.pixel(RADIUS as u32, RADIUS as u32);
    (a > 0).then_some(Color::rgba(r, g, b, 255))
}

/// `#RRGGBB` 形式的颜色值。
fn hex(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

/// 放大镜左上角的位置：默认在光标右下方；超出显示器右边或下边时翻到光标左边或上边，
/// 最后整体挪进显示器。
fn place(focus: Point, (width, height): (i32, i32), offset: i32, screen: Rect) -> (i32, i32) {
    let right = screen.x + screen.width as i32;
    let bottom = screen.y + screen.height as i32;
    let mut left = focus.x + offset;
    if left + width > right {
        left = focus.x - offset - width;
    }
    let mut top = focus.y + offset;
    if top + height > bottom {
        top = focus.y - offset - height;
    }
    (
        left.min(right - width).max(screen.x),
        top.min(bottom - height).max(screen.y),
    )
}
