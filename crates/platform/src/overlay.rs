//! 选区遮罩能力：铺满虚拟桌面的置顶界面。
//!
//! 分工：遮罩上的交互状态（拖拽框选、确定、取消……）和“画什么”都由 `core` 决定；
//! 平台只负责三件事——拍冻结帧、按 [`OverlayScene`] 绘制、把用户输入以
//! [`PlatformEvent::OverlayInput`](crate::PlatformEvent::OverlayInput) 递交回来。
//! 所有坐标都是虚拟桌面物理像素，与 [`crate::FrameSource`] 一致。

use crate::{Display, DisplayId, Image, Modifiers, PlatformError, Point, Rect};

/// 遮罩底下显示什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// 冻结帧：打开遮罩前先拍下每块显示器的画面，遮罩画在它上面；
    /// 框选结果从冻结帧裁剪（[`SelectionOverlay::frozen_image`]），不重新截图。
    FrozenFrame,
    /// 实时画面（录屏、长截屏）：不拍冻结帧，遮罩半透明地盖在仍在变化的桌面上，
    /// [`SelectionOverlay::frozen_image`] 返回错误。遮罩本身不会出现在任何屏幕捕获里。
    Live,
}

/// 选区遮罩能力。一次只能打开一个遮罩。
///
/// 典型顺序：[`open`](Self::open) → [`render`](Self::render)（首次调用时遮罩才出现，之后每次重画）
/// → 用户输入经平台事件循环递交 → …… → [`frozen_image`](Self::frozen_image) 取裁剪用的画面 →
/// [`close`](Self::close)。
pub trait SelectionOverlay {
    /// 打开遮罩：按 `backdrop` 准备背景（冻结帧模式下此刻拍下每块显示器的画面），
    /// 但还不显示，等第一次 [`render`](Self::render)。
    ///
    /// 返回拍下冻结帧那一刻的显示器布局；其中的 `DisplayId` 用于 [`frozen_image`](Self::frozen_image)。
    fn open(&mut self, backdrop: Backdrop) -> Result<Vec<Display>, PlatformError>;

    /// 按 `scene` 绘制遮罩。打开后的第一次调用会把遮罩显示出来并取得键盘焦点。
    fn render(&mut self, scene: &OverlayScene) -> Result<(), PlatformError>;

    /// 冻结帧中一块显示器的画面，尺寸等于它 `bounds` 的物理像素尺寸。只读取已拍下的冻结帧。
    fn frozen_image(&self, display: DisplayId) -> Result<Image, PlatformError>;

    /// 隐藏遮罩并释放冻结帧；没有打开时什么也不做。
    fn close(&mut self);
}

/// 遮罩上要画的全部内容（背景之上）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayScene {
    /// 按顺序从下往上画。
    pub items: Vec<OverlayItem>,
    /// 鼠标指针在遮罩上的形状。
    pub cursor: OverlayCursor,
}

impl OverlayScene {
    /// 场景中的全部文字标签，按绘制顺序排列。
    pub fn labels(&self) -> impl Iterator<Item = &Label> {
        self.items.iter().filter_map(|item| match item {
            OverlayItem::Label(label) => Some(label),
            _ => None,
        })
    }
}

/// 遮罩上的一个绘制元素。新的界面元素优先由现有元素组合；确实需要新画法时在这里加变体，
/// 再分别在各平台实现里画出来。
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayItem {
    /// 填充矩形，例如选区外的半透明遮挡。
    Fill { rect: Rect, color: Color },
    /// 紧贴在 `rect` 外侧、宽 `width` 像素的边框，不压住 `rect` 内的像素。
    Frame {
        rect: Rect,
        width: u32,
        color: Color,
    },
    /// 带底色的单行文字。
    Label(Label),
}

/// 带底色的单行文字。底色框的大小由平台按文字实际尺寸加上 `padding` 决定。
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    /// 底色框的定位点。
    pub anchor: Point,
    /// `anchor` 是底色框的哪个角。
    pub corner: Corner,
    /// 字号（物理像素），已按所在显示器的缩放比例换算。
    pub font_size: f32,
    /// 文字到底色框边缘的距离（物理像素）。
    pub padding: u32,
    pub color: Color,
    pub background: Color,
}

/// 矩形的一个角。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    BottomLeft,
}

/// sRGB 颜色，alpha 不预乘。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// 鼠标指针形状。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OverlayCursor {
    #[default]
    Crosshair,
    Arrow,
}

/// 用户在遮罩上的输入，坐标为虚拟桌面物理像素。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayInput {
    PointerMoved {
        position: Point,
        modifiers: Modifiers,
    },
    /// 按下鼠标键。`double_click` 表示这是一次双击的第二下（双击的第二下只递交这一个事件）。
    PointerPressed {
        button: PointerButton,
        position: Point,
        modifiers: Modifiers,
        double_click: bool,
    },
    PointerReleased {
        button: PointerButton,
        position: Point,
        modifiers: Modifiers,
    },
    KeyPressed {
        key: OverlayKey,
        modifiers: Modifiers,
    },
    /// 平台不得不中断这次遮罩，例如显示器布局变了、冻结帧已经失效。
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
}

/// 遮罩关心的按键，按需扩充；平台不递交这里没有的按键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKey {
    Escape,
    Enter,
}
