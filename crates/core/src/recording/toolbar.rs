//! 录屏控制条：布局、命中测试和每一刻要显示的内容（[`RecordingToolbarView`]）。
//!
//! 控制条从左到右是：录制指示点、已录时长 | 系统声音、麦克风 | 暂停 / 继续、停止（没有音轨时
//! 没有中间两个开关；开着的开关是白字，关着的是灰字加删除线）。它贴在区域边框外侧：优先在
//! 下方（与区域右边对齐，可以左右挪动），下方放不下时放到上方，上下都放不下时放到右侧、左侧
//! （与区域下边对齐，可以上下挪动）；四周都放不下（例如录制整块显示器）时放在区域内的右下角——
//! 平台保证控制条不会被录进视频，只是挡住了这一小块。控制条总是整个落在某一块显示器以内，
//! 尺寸按区域右下角所在显示器的缩放比例放大。
//!
//! 按钮宽度由 `core` 按文字估算（与标注工具条相同），命中测试不依赖平台的文字排版；
//! “暂停”按钮按“暂停”“继续”中较宽的一个留出宽度，切换时控制条不变大小。
//!
//! 窗口录屏（#21）复用同一套布局：把 `region` 换成窗口此刻的矩形重新 [`Toolbar::beside`]，
//! 控制条和边框就跟着窗口走。

use std::time::Duration;

use bps_platform::{
    Color, Display, OverlayInput, OverlayItem, Platform, Point, PointerButton,
    RecordingToolbarView, Rect, RegionBorder,
};

use super::AudioInput;
use crate::Texts;

/// 控制条上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    /// 系统声音或麦克风的开关。
    Audio(AudioInput),
    /// 暂停（录制中）或继续（已暂停）。
    PauseResume,
    Stop,
}

/// 两个声音开关此刻的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioSwitches {
    pub system: bool,
    pub microphone: bool,
}

/// 以下尺寸是 100% 缩放下的值（物理像素），与标注工具条一致。
const FONT_SIZE: f64 = 14.0;
const BUTTON_HEIGHT: f64 = 30.0;
const BUTTON_PADDING: f64 = 10.0;
const BUTTON_GAP: f64 = 2.0;
const TOOLBAR_PADDING: f64 = 4.0;
const SEPARATOR_MARGIN: f64 = 6.0;
/// 录制指示点所占的宽度。
const INDICATOR_WIDTH: f64 = 24.0;
/// 控制条与区域边框之间的距离。
const GAP: f64 = 6.0;
/// 区域边框的宽度。
const BORDER_WIDTH: f64 = 2.0;
/// 计时文字按这个最长的样子留出宽度。
const WIDEST_TIME: &str = "00:00:00";
const INDICATOR: &str = "●";

const BACKGROUND: Color = Color::rgba(32, 32, 32, 240);
const OUTLINE: Color = Color::rgba(255, 255, 255, 48);
const SEPARATOR: Color = Color::rgba(255, 255, 255, 64);
const HOVER: Color = Color::rgba(255, 255, 255, 36);
const TEXT: Color = Color::rgba(255, 255, 255, 255);
/// 录制中：指示点和区域边框是红色。
const RECORDING: Color = Color::rgba(240, 40, 40, 255);
/// 已暂停：指示点和区域边框是琥珀色。
const PAUSED: Color = Color::rgba(255, 176, 0, 255);
/// 关着的声音开关的文字和删除线。
const SWITCHED_OFF: Color = Color::rgba(255, 255, 255, 110);

/// 一次布局好的控制条。
pub(crate) struct Toolbar {
    bounds: Rect,
    indicator: Rect,
    time: Rect,
    separators: Vec<Rect>,
    /// 声音开关（按钮范围、文字宽度、文字）；没有音轨时为空。
    switches: Vec<(AudioInput, Rect, i32, String)>,
    pause: Rect,
    stop: Rect,
    font_size: f32,
    outline: u32,
    /// 区域边框（围住的范围与宽度）；边框完全落在显示器以外时为 `None`。
    border: Option<(Rect, u32)>,
    pause_label: String,
    resume_label: String,
    stop_label: String,
}

/// 控制条此刻的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolbarState {
    /// 已录时长（不含暂停的时间）。
    pub recorded: Duration,
    pub paused: bool,
    /// 鼠标指针所在的按钮。
    pub hovered: Option<Button>,
    /// 声音开关；没有音轨时为 `None`。
    pub audio: Option<AudioSwitches>,
}

impl Toolbar {
    /// 贴着录制范围 `region` 的控制条，文字用 `texts` 的语言；`audio` 表示有音轨（显示声音开关）。
    pub fn beside(region: Rect, displays: &[Display], texts: &Texts, audio: bool) -> Self {
        let corner = Point::new(right(region) - 1, bottom(region) - 1);
        let scale = displays
            .iter()
            .find(|d| d.bounds.contains(corner))
            .or_else(|| {
                displays.iter().max_by_key(|d| {
                    d.bounds
                        .intersection(region)
                        .map_or(0, |r| u64::from(r.width) * u64::from(r.height))
                })
            })
            .map_or(1.0, |d| d.scale_factor);
        let px = |v: f64| (v * scale).round() as i32;
        let font_size = (FONT_SIZE * scale) as f32;
        let text = |label: &str| text_width(label, f64::from(font_size)).ceil() as i32;
        let pause_label = texts.get("recording-toolbar-pause");
        let resume_label = texts.get("recording-toolbar-resume");
        let stop_label = texts.get("recording-toolbar-stop");

        // 先以 (0, 0) 为左上角排好，再整体移到合适的位置。
        let (padding, height) = (px(TOOLBAR_PADDING), px(BUTTON_HEIGHT));
        let button_padding = px(BUTTON_PADDING);
        let cell = |x: i32, width: i32| Rect::new(x, padding, width as u32, height as u32);
        let indicator = cell(padding, px(INDICATOR_WIDTH));
        let time = cell(right(indicator), text(WIDEST_TIME) + button_padding);
        let margin = px(SEPARATOR_MARGIN);
        let line = px(1.0).max(1);
        let separator_at = |x: i32| {
            Rect::new(
                x + margin,
                padding + margin,
                line as u32,
                (height - 2 * margin).max(1) as u32,
            )
        };
        let mut separators = vec![separator_at(right(time))];
        let mut switches = Vec::new();
        if audio {
            let mut x = right(separators[0]) + margin;
            for (input, key) in [
                (AudioInput::System, "recording-toolbar-system-audio"),
                (AudioInput::Microphone, "recording-toolbar-microphone"),
            ] {
                let label = texts.get(key);
                let width = text(&label);
                let rect = cell(x, (width + 2 * button_padding).max(height));
                x = right(rect) + px(BUTTON_GAP);
                switches.push((input, rect, width, label));
            }
            separators.push(separator_at(x - px(BUTTON_GAP)));
        }
        let last_separator = *separators.last().expect("至少有一条分隔线");
        let pause_width = text(&pause_label).max(text(&resume_label)) + 2 * button_padding;
        let pause = cell(right(last_separator) + margin, pause_width.max(height));
        let stop = cell(
            right(pause) + px(BUTTON_GAP),
            (text(&stop_label) + 2 * button_padding).max(height),
        );
        let size = (right(stop) + padding, height + 2 * padding);

        let border_width = px(BORDER_WIDTH).max(1);
        let screens: Vec<Rect> = displays.iter().map(|d| d.bounds).collect();
        let (left, top) = place(region, size, px(GAP) + border_width, &screens);
        let shift = |r: Rect| Rect::new(r.x + left, r.y + top, r.width, r.height);
        let border_visible = screens.iter().any(|screen| {
            screen
                .intersection(grow(region, border_width))
                .is_some_and(|part| region.intersection(part) != Some(part))
        });
        Self {
            bounds: Rect::new(left, top, size.0 as u32, size.1 as u32),
            indicator: shift(indicator),
            time: shift(time),
            separators: separators.into_iter().map(shift).collect(),
            switches: switches
                .into_iter()
                .map(|(input, rect, width, label)| (input, shift(rect), width, label))
                .collect(),
            pause: shift(pause),
            stop: shift(stop),
            font_size,
            outline: line as u32,
            border: border_visible.then_some((region, border_width as u32)),
            pause_label,
            resume_label,
            stop_label,
        }
    }

    /// `point` 处的按钮。
    pub fn button_at(&self, point: Point) -> Option<Button> {
        if let Some((input, ..)) = self.switches.iter().find(|s| s.1.contains(point)) {
            Some(Button::Audio(*input))
        } else if self.pause.contains(point) {
            Some(Button::PauseResume)
        } else if self.stop.contains(point) {
            Some(Button::Stop)
        } else {
            None
        }
    }

    /// 控制条在 `state` 时的样子。
    pub fn view(&self, state: ToolbarState) -> RecordingToolbarView {
        let accent = if state.paused { PAUSED } else { RECORDING };
        let text = |rect: Rect, text: &str, color: Color| OverlayItem::Text {
            rect,
            text: text.to_owned(),
            font_size: self.font_size,
            color,
        };
        let mut items = vec![
            OverlayItem::Fill {
                rect: self.bounds,
                color: BACKGROUND,
            },
            OverlayItem::Frame {
                rect: self.bounds,
                width: self.outline,
                color: OUTLINE,
            },
            text(self.indicator, INDICATOR, accent),
            text(self.time, &format_time(state.recorded), TEXT),
        ];
        items.extend(self.separators.iter().map(|&rect| OverlayItem::Fill {
            rect,
            color: SEPARATOR,
        }));
        let switches = state.audio.unwrap_or(AudioSwitches {
            system: false,
            microphone: false,
        });
        for (input, rect, width, label) in &self.switches {
            let button = Button::Audio(*input);
            if state.hovered == Some(button) {
                items.push(OverlayItem::Fill {
                    rect: *rect,
                    color: HOVER,
                });
            }
            let on = match input {
                AudioInput::System => switches.system,
                AudioInput::Microphone => switches.microphone,
            };
            items.push(text(*rect, label, if on { TEXT } else { SWITCHED_OFF }));
            if !on {
                // 删除线：横穿文字中间。
                let center = rect.x + rect.width as i32 / 2;
                items.push(OverlayItem::Fill {
                    rect: Rect::new(
                        center - width / 2,
                        rect.y + rect.height as i32 / 2,
                        (*width).max(1) as u32,
                        self.outline,
                    ),
                    color: SWITCHED_OFF,
                });
            }
        }
        let pause_label = if state.paused {
            &self.resume_label
        } else {
            &self.pause_label
        };
        for (button, rect, label) in [
            (Button::PauseResume, self.pause, pause_label),
            (Button::Stop, self.stop, &self.stop_label),
        ] {
            if state.hovered == Some(button) {
                items.push(OverlayItem::Fill { rect, color: HOVER });
            }
            items.push(text(rect, label, TEXT));
        }
        RecordingToolbarView {
            bounds: self.bounds,
            items,
            border: self.border.map(|(rect, width)| RegionBorder {
                rect,
                width,
                color: accent,
            }),
        }
    }
}

/// 录制期间控制条上的交互：悬停高亮、按钮点击，以及把变化了的样子交给平台显示。
pub(crate) struct Controls {
    toolbar: Toolbar,
    hovered: Option<Button>,
    /// 按下鼠标左键时所在的按钮；在同一个按钮上松开才算点击。
    pressed: Option<Button>,
    /// 平台上正显示着的样子；没变时不重画。
    shown: Option<RecordingToolbarView>,
    /// 显示失败过：之后不再尝试（录制照常进行，用热键停止）。
    broken: bool,
}

impl Controls {
    pub fn new(toolbar: Toolbar) -> Self {
        Self {
            toolbar,
            hovered: None,
            pressed: None,
            shown: None,
            broken: false,
        }
    }

    /// 处理控制条上的一次输入，返回被点击的按钮。
    pub fn handle(&mut self, input: &OverlayInput) -> Option<Button> {
        match *input {
            OverlayInput::PointerMoved { position, .. } => {
                self.hovered = self.toolbar.button_at(position);
                None
            }
            OverlayInput::PointerPressed {
                button: PointerButton::Left,
                position,
                ..
            } => {
                self.pressed = self.toolbar.button_at(position);
                self.hovered = self.pressed;
                None
            }
            OverlayInput::PointerReleased {
                button: PointerButton::Left,
                position,
                ..
            } => {
                let pressed = self.pressed.take()?;
                (self.toolbar.button_at(position) == Some(pressed)).then_some(pressed)
            }
            _ => None,
        }
    }

    /// 按此刻的状态显示控制条；与正显示着的样子相同时什么也不做。显示失败时隐藏控制条，
    /// 不影响录制。
    pub fn refresh(
        &mut self,
        platform: &mut Platform,
        recorded: Duration,
        paused: bool,
        audio: Option<AudioSwitches>,
    ) {
        if self.broken {
            return;
        }
        let view = self.toolbar.view(ToolbarState {
            recorded,
            paused,
            hovered: self.hovered,
            audio,
        });
        if self.shown.as_ref() == Some(&view) {
            return;
        }
        match platform.recording_toolbar.show(&view) {
            Ok(()) => self.shown = Some(view),
            Err(_) => {
                self.broken = true;
                platform.recording_toolbar.hide();
            }
        }
    }
}

/// 已录时长：不到一小时是“分:秒”，否则是“时:分:秒”，秒向下取整。
fn format_time(recorded: Duration) -> String {
    let seconds = recorded.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

/// 控制条左上角的位置，规则见模块文档。`gap` 是控制条与区域之间的距离（含区域边框）。
fn place(region: Rect, (width, height): (i32, i32), gap: i32, screens: &[Rect]) -> (i32, i32) {
    /// 候选位置可以沿哪个方向挪进显示器。
    #[derive(Clone, Copy)]
    enum Slide {
        Horizontal,
        Vertical,
    }
    let clamp = |v: i32, low: i32, high: i32| v.min(high).max(low);
    let overlaps = |a: (i32, i32), b: (i32, i32)| a.0 < b.1 && b.0 < a.1;
    let candidates = [
        (
            right(region) - width,
            bottom(region) + gap,
            Slide::Horizontal,
        ),
        (
            right(region) - width,
            region.y - gap - height,
            Slide::Horizontal,
        ),
        (
            right(region) + gap,
            bottom(region) - height,
            Slide::Vertical,
        ),
        (
            region.x - gap - width,
            bottom(region) - height,
            Slide::Vertical,
        ),
    ];
    for (x, y, slide) in candidates {
        for screen in screens {
            // 只挪进与区域同一列（上下两侧）或同一行（左右两侧）的显示器。
            let (x, y) = match slide {
                Slide::Horizontal => {
                    if !overlaps((screen.x, right(*screen)), (region.x, right(region))) {
                        continue;
                    }
                    (clamp(x, screen.x, right(*screen) - width), y)
                }
                Slide::Vertical => {
                    if !overlaps((screen.y, bottom(*screen)), (region.y, bottom(region))) {
                        continue;
                    }
                    (x, clamp(y, screen.y, bottom(*screen) - height))
                }
            };
            let rect = Rect::new(x, y, width as u32, height as u32);
            if screen.intersection(rect) == Some(rect) {
                return (x, y);
            }
        }
    }
    // 四周都放不下：放在区域内的右下角，挪进区域右下角所在的显示器。
    let corner = Point::new(right(region) - 1, bottom(region) - 1);
    let screen = screens
        .iter()
        .find(|s| s.contains(corner))
        .copied()
        .unwrap_or(region);
    (
        clamp(right(region) - gap - width, screen.x, right(screen) - width),
        clamp(
            bottom(region) - gap - height,
            screen.y,
            bottom(screen) - height,
        ),
    )
}

/// 估算一行文字的宽度（像素）：汉字等全角字符占 1 个字号，其余占 0.6 个字号（同标注工具条）。
fn text_width(text: &str, font_size: f64) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 0.6 } else { 1.0 })
        .sum::<f64>()
        * font_size
}

/// 向四周各扩大 `by` 像素。
fn grow(rect: Rect, by: i32) -> Rect {
    Rect::new(
        rect.x - by,
        rect.y - by,
        (rect.width as i32 + 2 * by) as u32,
        (rect.height as i32 + 2 * by) as u32,
    )
}

fn right(rect: Rect) -> i32 {
    rect.x + rect.width as i32
}

fn bottom(rect: Rect) -> i32 {
    rect.y + rect.height as i32
}
