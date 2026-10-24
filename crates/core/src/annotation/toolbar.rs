//! 标注工具条：按钮的布局、命中测试和绘制。
//!
//! 工具条有两行：上一行是标注工具和结束方式，下一行是之后画的标注用的颜色和线宽。
//! 工具条贴在区域外侧的右下方（与区域右边对齐）；下方放不下时放到上方，上下都放不下时放在
//! 区域内的右下角；整条工具条始终在区域右下角所在的那块显示器以内。尺寸按那块显示器的
//! 缩放比例放大。
//!
//! 按钮宽度由 `core` 按文字估算（平台只负责把文字居中画在按钮里），这样命中测试不依赖平台
//! 的文字排版。扩展点：在 [`TOP_ROW`] / [`style_row`] 里加按钮，在 [`Button`] 加变体，
//! 按钮文字放进两份 FTL。

use bps_platform::{Color, Display, OverlayItem, Point, Rect};

use super::style::{PALETTE, Thickness};
use crate::Texts;

/// 标注工具：选中后在区域里拖拽画出框、箭头、马赛克，或单击后输入文字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Rectangle,
    Arrow,
    Mosaic,
    Text,
}

/// 工具条上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Tool(Tool),
    /// 之后画的标注用这种颜色。
    Color(Color),
    /// 之后画的标注用这一档线宽。
    Thickness(Thickness),
    /// 取消这次捕获。
    Cancel,
    /// 结束方式：另存为。
    SaveAs,
    /// 结束方式：自动存储。
    AutoSave,
    /// 结束方式：复制到剪贴板。
    Copy,
}

impl Button {
    /// 按钮文字在文本资源里的消息；色块和线宽按钮没有文字。
    fn message(self) -> Option<&'static str> {
        Some(match self {
            Button::Tool(Tool::Rectangle) => "annotation-rectangle",
            Button::Tool(Tool::Arrow) => "annotation-arrow",
            Button::Tool(Tool::Mosaic) => "annotation-mosaic",
            Button::Tool(Tool::Text) => "annotation-text",
            Button::Cancel => "annotation-cancel",
            Button::SaveAs => "annotation-save-as",
            Button::AutoSave => "annotation-auto-save",
            Button::Copy => "annotation-copy",
            Button::Color(_) | Button::Thickness(_) => return None,
        })
    }
}

#[derive(Clone, Copy)]
enum Entry {
    Button(Button),
    Separator,
}

/// 上一行从左到右：工具 | 取消、结束方式（最常用的“复制”在最右边）。
const TOP_ROW: [Entry; 9] = [
    Entry::Button(Button::Tool(Tool::Rectangle)),
    Entry::Button(Button::Tool(Tool::Arrow)),
    Entry::Button(Button::Tool(Tool::Mosaic)),
    Entry::Button(Button::Tool(Tool::Text)),
    Entry::Separator,
    Entry::Button(Button::Cancel),
    Entry::Button(Button::SaveAs),
    Entry::Button(Button::AutoSave),
    Entry::Button(Button::Copy),
];

/// 下一行从左到右：预设颜色 | 三档线宽（从细到粗）。
fn style_row() -> Vec<Entry> {
    PALETTE
        .iter()
        .map(|&color| Entry::Button(Button::Color(color)))
        .chain([Entry::Separator])
        .chain(
            Thickness::ALL
                .iter()
                .map(|&thickness| Entry::Button(Button::Thickness(thickness))),
        )
        .collect()
}

/// 以下尺寸是 100% 缩放下的值（物理像素）。
const FONT_SIZE: f64 = 14.0;
const BUTTON_HEIGHT: f64 = 30.0;
const BUTTON_PADDING: f64 = 10.0;
const BUTTON_GAP: f64 = 2.0;
const TOOLBAR_PADDING: f64 = 4.0;
const SEPARATOR_MARGIN: f64 = 6.0;
/// 色块离按钮上下边缘的距离。
const SWATCH_INSET: f64 = 7.0;
/// 线宽按钮的宽度和里面线条样本的长度。
const THICKNESS_BUTTON_WIDTH: f64 = 36.0;
const THICKNESS_SAMPLE_LENGTH: f64 = 20.0;
/// 工具条与区域边框之间的距离。
const GAP: f64 = 6.0;

const BACKGROUND: Color = Color::rgba(32, 32, 32, 240);
const BORDER: Color = Color::rgba(255, 255, 255, 48);
const SEPARATOR: Color = Color::rgba(255, 255, 255, 64);
const HOVER: Color = Color::rgba(255, 255, 255, 36);
/// 选中的工具和线宽，与选区边框同色。
const ACTIVE: Color = Color::rgba(30, 144, 255, 255);
const TEXT: Color = Color::rgba(255, 255, 255, 255);
/// 色块的描边（让黑色色块在深色工具条上也看得见）和选中色块的描边。
const SWATCH_BORDER: Color = Color::rgba(255, 255, 255, 96);
const SWATCH_SELECTED: Color = Color::rgba(255, 255, 255, 255);

/// 工具条上一个按钮的位置和内容。
struct Placed {
    button: Button,
    rect: Rect,
    /// 按钮上的文字（工具、结束方式）。
    label: Option<String>,
}

/// 工具条当前的状态：选中的工具、颜色、线宽和鼠标悬停的按钮。
pub(crate) struct ToolbarState {
    pub tool: Option<Tool>,
    pub color: Color,
    pub thickness: Thickness,
    pub hovered: Option<Button>,
}

/// 一次布局好的工具条。
pub(crate) struct Toolbar {
    bounds: Rect,
    buttons: Vec<Placed>,
    separators: Vec<Rect>,
    font_size: f32,
    /// 所在显示器的缩放比例，用于色块、线条样本等的尺寸。
    scale: f64,
    border: u32,
}

impl Toolbar {
    /// 贴着 `region`（外侧有宽 `frame` 的边框）的工具条，按钮文字用 `texts` 的语言。
    pub fn beside(region: Rect, frame: u32, displays: &[Display], texts: &Texts) -> Self {
        let corner = Point::new(
            region.x + region.width as i32 - 1,
            region.y + region.height as i32 - 1,
        );
        let display = displays
            .iter()
            .find(|d| d.bounds.contains(corner))
            .or_else(|| {
                displays.iter().max_by_key(|d| {
                    d.bounds
                        .intersection(region)
                        .map_or(0, |r| u64::from(r.width) * u64::from(r.height))
                })
            });
        let (screen, scale) = display.map_or((region, 1.0), |d| (d.bounds, d.scale_factor));
        let px = |v: f64| (v * scale).round() as i32;
        let font_size = (FONT_SIZE * scale) as f32;

        // 先以 (0, 0) 为左上角排好，再整体移到合适的位置。
        let (padding, height) = (px(TOOLBAR_PADDING), px(BUTTON_HEIGHT));
        let mut buttons = Vec::new();
        let mut separators = Vec::new();
        let mut width = 0;
        for (row, entries) in [TOP_ROW.to_vec(), style_row()].iter().enumerate() {
            let top = padding + row as i32 * (height + px(BUTTON_GAP));
            let mut x = padding;
            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    x += px(BUTTON_GAP);
                }
                match *entry {
                    Entry::Button(button) => {
                        let label = button.message().map(|m| texts.get(m));
                        let button_width = match (&label, button) {
                            (Some(label), _) => {
                                let text = text_width(label, f64::from(font_size));
                                (text.ceil() as i32 + 2 * px(BUTTON_PADDING)).max(height)
                            }
                            (None, Button::Thickness(_)) => px(THICKNESS_BUTTON_WIDTH),
                            (None, _) => height,
                        };
                        buttons.push(Placed {
                            button,
                            rect: Rect::new(x, top, button_width as u32, height as u32),
                            label,
                        });
                        x += button_width;
                    }
                    Entry::Separator => {
                        let margin = px(SEPARATOR_MARGIN);
                        let line = px(1.0).max(1);
                        separators.push(Rect::new(
                            x + margin,
                            top + margin,
                            line as u32,
                            (height - 2 * margin).max(1) as u32,
                        ));
                        x += 2 * margin + line;
                    }
                }
            }
            width = width.max(x + padding);
        }
        let size = (width, 2 * height + px(BUTTON_GAP) + 2 * padding);

        let gap = px(GAP) + frame as i32;
        let (left, top) = place(region, size, gap, screen);
        let shift = |r: Rect| Rect::new(r.x + left, r.y + top, r.width, r.height);
        Self {
            bounds: Rect::new(left, top, size.0 as u32, size.1 as u32),
            buttons: buttons
                .into_iter()
                .map(|placed| Placed {
                    rect: shift(placed.rect),
                    ..placed
                })
                .collect(),
            separators: separators.into_iter().map(shift).collect(),
            font_size,
            scale,
            border: px(1.0).max(1) as u32,
        }
    }

    /// `point` 是否在工具条上（包括按钮之间的空隙）。
    pub fn contains(&self, point: Point) -> bool {
        self.bounds.contains(point)
    }

    /// `point` 处的按钮。
    pub fn button_at(&self, point: Point) -> Option<Button> {
        self.buttons
            .iter()
            .find(|placed| placed.rect.contains(point))
            .map(|placed| placed.button)
    }

    /// 画工具条的遮罩元素，从下往上。
    pub fn items(&self, state: &ToolbarState) -> Vec<OverlayItem> {
        let px = |v: f64| ((v * self.scale).round() as i32).max(1);
        let mut items = vec![
            OverlayItem::Fill {
                rect: self.bounds,
                color: BACKGROUND,
            },
            OverlayItem::Frame {
                rect: self.bounds,
                width: self.border,
                color: BORDER,
            },
        ];
        items.extend(self.separators.iter().map(|&rect| OverlayItem::Fill {
            rect,
            color: SEPARATOR,
        }));
        for placed in &self.buttons {
            let (button, rect) = (placed.button, placed.rect);
            let active = match button {
                Button::Tool(tool) => state.tool == Some(tool),
                Button::Thickness(thickness) => state.thickness == thickness,
                _ => false,
            };
            let highlight = if active {
                Some(ACTIVE)
            } else if state.hovered == Some(button) {
                Some(HOVER)
            } else {
                None
            };
            if let Some(color) = highlight {
                items.push(OverlayItem::Fill { rect, color });
            }
            match (&placed.label, button) {
                (Some(label), _) => items.push(OverlayItem::Text {
                    rect,
                    text: label.clone(),
                    font_size: self.font_size,
                    color: TEXT,
                }),
                (None, Button::Color(color)) => {
                    let inset = px(SWATCH_INSET);
                    let side = (rect.height as i32 - 2 * inset).max(1);
                    let swatch = Rect::new(
                        rect.x + (rect.width as i32 - side) / 2,
                        rect.y + inset,
                        side as u32,
                        side as u32,
                    );
                    items.push(OverlayItem::Fill {
                        rect: swatch,
                        color,
                    });
                    let (width, border) = if state.color == color {
                        (px(2.0) as u32, SWATCH_SELECTED)
                    } else {
                        (self.border, SWATCH_BORDER)
                    };
                    items.push(OverlayItem::Frame {
                        rect: swatch,
                        width,
                        color: border,
                    });
                }
                (None, Button::Thickness(thickness)) => {
                    let length = px(THICKNESS_SAMPLE_LENGTH);
                    let thick = px(thickness.sample_height());
                    items.push(OverlayItem::Fill {
                        rect: Rect::new(
                            rect.x + (rect.width as i32 - length) / 2,
                            rect.y + (rect.height as i32 - thick) / 2,
                            length as u32,
                            thick as u32,
                        ),
                        color: TEXT,
                    });
                }
                (None, _) => {}
            }
        }
        items
    }
}

/// 工具条左上角的位置：优先在区域下方、与区域右边对齐；下方放不下时放到上方，都放不下时
/// 放在区域内的右下角；最后整体挪进 `screen`。
fn place(region: Rect, (width, height): (i32, i32), gap: i32, screen: Rect) -> (i32, i32) {
    let screen_right = screen.x + screen.width as i32;
    let screen_bottom = screen.y + screen.height as i32;
    let region_right = region.x + region.width as i32;
    let region_bottom = region.y + region.height as i32;
    let below = region_bottom + gap;
    let above = region.y - gap - height;
    let top = if below + height <= screen_bottom {
        below
    } else if above >= screen.y {
        above
    } else {
        region_bottom - gap - height
    };
    let left = region_right - width;
    let clamp = |v: i32, low: i32, high: i32| v.min(high).max(low);
    (
        clamp(left, screen.x, screen_right - width),
        clamp(top, screen.y, screen_bottom - height),
    )
}

/// 估算一行文字的宽度（像素）：汉字等全角字符占 1 个字号，其余占 0.6 个字号。
fn text_width(text: &str, font_size: f64) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 0.6 } else { 1.0 })
        .sum::<f64>()
        * font_size
}
