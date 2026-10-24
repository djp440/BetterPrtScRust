//! 录屏控制条能力：录屏期间悬浮在桌面上的录屏控制条和区域边框。
//!
//! 分工与选区遮罩相同：控制条放在哪、上面画什么、按钮的命中测试都由 `core` 决定；平台只负责
//! 按 [`RecordingToolbarView`] 显示，并把用户在控制条上的鼠标输入以
//! [`PlatformEvent::OverlayInput`](crate::PlatformEvent::OverlayInput) 递交回来。
//! 坐标都是虚拟桌面物理像素，与选区遮罩一致。
//!
//! 平台保证：
//! - 控制条和区域边框不出现在任何屏幕捕获里（包括本程序的录屏），所以即使它们落在录制范围内
//!   （例如录制整块显示器）也不会被录进视频；
//! - 不抢键盘焦点：点控制条时正在录制的程序照常保持前台、接收键盘输入；
//! - 控制条范围以外（包括区域边框）不拦截鼠标，用户照常操作底下的窗口。

use crate::{Color, OverlayItem, PlatformError, Rect};

/// 录屏控制条与区域边框。
pub trait RecordingToolbar {
    /// 显示或更新控制条与区域边框：第一次调用时出现，之后按 `view` 重画（位置、大小都可以变）。
    fn show(&mut self, view: &RecordingToolbarView) -> Result<(), PlatformError>;

    /// 隐藏控制条与区域边框并释放资源；没有显示时什么也不做。
    fn hide(&mut self);
}

/// 录屏控制条和区域边框此刻的样子。
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingToolbarView {
    /// 控制条的范围；只有这个范围内接收鼠标输入。
    pub bounds: Rect,
    /// 控制条上的内容，按顺序从下往上画；超出 `bounds` 的部分不显示。
    pub items: Vec<OverlayItem>,
    /// 围住录制范围的边框；`None` 表示不画（例如录制范围就是整块显示器）。
    pub border: Option<RegionBorder>,
}

impl RecordingToolbarView {
    /// 控制条上内容为 `text` 的 [`OverlayItem::Text`] 所在的矩形（有多个时取最上面的）。
    pub fn text_rect(&self, text: &str) -> Option<Rect> {
        self.items.iter().rev().find_map(|item| match item {
            OverlayItem::Text { rect, text: t, .. } if t == text => Some(*rect),
            _ => None,
        })
    }

    /// 控制条上的全部文字（[`OverlayItem::Text`] 与 [`OverlayItem::Label`]），按绘制顺序排列。
    pub fn texts(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|item| match item {
                OverlayItem::Text { text, .. } => Some(text.as_str()),
                OverlayItem::Label(label) => Some(label.text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// 区域边框：紧贴在 `rect` 外侧、宽 `width` 像素，不压住 `rect` 内的像素。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionBorder {
    pub rect: Rect,
    pub width: u32,
    pub color: Color,
}
