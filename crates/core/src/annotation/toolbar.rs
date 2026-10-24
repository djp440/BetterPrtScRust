//! 标注工具条：按钮的布局、命中测试和绘制。
//!
//! 工具条贴在区域外侧的右下方（与区域右边对齐）；下方放不下时放到上方，上下都放不下时放在
//! 区域内的右下角；整条工具条始终在区域右下角所在的那块显示器以内。尺寸按那块显示器的
//! 缩放比例放大。
//!
//! 按钮宽度由 `core` 按文字估算（平台只负责把文字居中画在按钮里），这样命中测试不依赖平台
//! 的文字排版。扩展点：在 [`ENTRIES`] 里加按钮（#13 的颜色、线宽），在 [`Button`] 加变体。

use bps_platform::{Color, Display, OverlayItem, Point, Rect};

/// 标注工具：选中后在区域里拖拽画出对应的标注。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Rectangle,
    Arrow,
}

/// 工具条上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Tool(Tool),
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
    fn label(self) -> &'static str {
        match self {
            Button::Tool(Tool::Rectangle) => "矩形",
            Button::Tool(Tool::Arrow) => "箭头",
            Button::Cancel => "取消",
            Button::SaveAs => "另存为",
            Button::AutoSave => "自动存储",
            Button::Copy => "复制",
        }
    }
}

#[derive(Clone, Copy)]
enum Entry {
    Button(Button),
    Separator,
}

/// 工具条从左到右的内容：工具 | 取消、结束方式（最常用的“复制”在最右边）。
const ENTRIES: [Entry; 7] = [
    Entry::Button(Button::Tool(Tool::Rectangle)),
    Entry::Button(Button::Tool(Tool::Arrow)),
    Entry::Separator,
    Entry::Button(Button::Cancel),
    Entry::Button(Button::SaveAs),
    Entry::Button(Button::AutoSave),
    Entry::Button(Button::Copy),
];

/// 以下尺寸是 100% 缩放下的值（物理像素）。
const FONT_SIZE: f64 = 14.0;
const BUTTON_HEIGHT: f64 = 30.0;
const BUTTON_PADDING: f64 = 10.0;
const BUTTON_GAP: f64 = 2.0;
const TOOLBAR_PADDING: f64 = 4.0;
const SEPARATOR_MARGIN: f64 = 6.0;
/// 工具条与区域边框之间的距离。
const GAP: f64 = 6.0;

const BACKGROUND: Color = Color::rgba(32, 32, 32, 240);
const BORDER: Color = Color::rgba(255, 255, 255, 48);
const SEPARATOR: Color = Color::rgba(255, 255, 255, 64);
const HOVER: Color = Color::rgba(255, 255, 255, 36);
/// 选中的工具，与选区边框同色。
const ACTIVE: Color = Color::rgba(30, 144, 255, 255);
const TEXT: Color = Color::rgba(255, 255, 255, 255);

/// 一次布局好的工具条。
pub(crate) struct Toolbar {
    bounds: Rect,
    buttons: Vec<(Button, Rect)>,
    separators: Vec<Rect>,
    font_size: f32,
    border: u32,
}

impl Toolbar {
    /// 贴着 `region`（外侧有宽 `frame` 的边框）的工具条。
    pub fn beside(region: Rect, frame: u32, displays: &[Display]) -> Self {
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
        let mut x = padding;
        let mut buttons = Vec::new();
        let mut separators = Vec::new();
        for (index, entry) in ENTRIES.iter().enumerate() {
            if index > 0 {
                x += px(BUTTON_GAP);
            }
            match *entry {
                Entry::Button(button) => {
                    let text = text_width(button.label(), f64::from(font_size));
                    let width = (text.ceil() as i32 + 2 * px(BUTTON_PADDING)).max(height);
                    buttons.push((button, Rect::new(x, padding, width as u32, height as u32)));
                    x += width;
                }
                Entry::Separator => {
                    let margin = px(SEPARATOR_MARGIN);
                    let width = px(1.0).max(1);
                    separators.push(Rect::new(
                        x + margin,
                        padding + margin,
                        width as u32,
                        (height - 2 * margin).max(1) as u32,
                    ));
                    x += 2 * margin + width;
                }
            }
        }
        let size = (x + padding, height + 2 * padding);

        let gap = px(GAP) + frame as i32;
        let (left, top) = place(region, size, gap, screen);
        let shift = |r: Rect| Rect::new(r.x + left, r.y + top, r.width, r.height);
        Self {
            bounds: Rect::new(left, top, size.0 as u32, size.1 as u32),
            buttons: buttons.into_iter().map(|(b, r)| (b, shift(r))).collect(),
            separators: separators.into_iter().map(shift).collect(),
            font_size,
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
            .find(|(_, rect)| rect.contains(point))
            .map(|(button, _)| *button)
    }

    /// 画工具条的遮罩元素，从下往上。
    pub fn items(&self, active: Option<Tool>, hovered: Option<Button>) -> Vec<OverlayItem> {
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
        for &(button, rect) in &self.buttons {
            let highlight = if active.is_some_and(|tool| button == Button::Tool(tool)) {
                Some(ACTIVE)
            } else if hovered == Some(button) {
                Some(HOVER)
            } else {
                None
            };
            if let Some(color) = highlight {
                items.push(OverlayItem::Fill { rect, color });
            }
            items.push(OverlayItem::Text {
                rect,
                text: button.label().to_owned(),
                font_size: self.font_size,
                color: TEXT,
            });
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
