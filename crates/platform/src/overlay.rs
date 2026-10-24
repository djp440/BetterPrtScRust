//! 选区遮罩能力：铺满虚拟桌面的置顶界面。
//!
//! 分工：遮罩上的交互状态（拖拽框选、确定、取消……）和“画什么”都由 `core` 决定；
//! 平台只负责三件事——拍冻结帧、按 [`OverlayScene`] 绘制、把用户输入以
//! [`PlatformEvent::OverlayInput`](crate::PlatformEvent::OverlayInput) 递交回来。
//! 所有坐标都是虚拟桌面物理像素，与 [`crate::FrameSource`] 一致。

use crate::{Display, DisplayId, Image, Modifiers, PlatformError, Point, Rect, TextMask};

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
///
/// 每一帧从下往上是：背景（冻结帧）→ 画布（[`show_canvas`](Self::show_canvas)，可选）→
/// 场景（[`OverlayScene`]）。
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

    /// 冻结帧中一块显示器画面的一小块（例如放大镜要显示的像素）。`area` 是这块显示器画面内的
    /// 像素坐标（画面左上角为 (0, 0)），必须整个落在画面内。只读取已拍下的冻结帧。
    ///
    /// 默认实现取出整幅画面再裁剪；能只读取这一小块的平台应当覆盖它（放大镜每移动一次指针
    /// 都会读取）。
    fn frozen_pixels(&self, display: DisplayId, area: Rect) -> Result<Image, PlatformError> {
        let image = self.frozen_image(display)?;
        image.crop(area).ok_or_else(|| {
            PlatformError::new(format!(
                "{area:?} 超出了 {}x{} 的冻结帧画面",
                image.width(),
                image.height()
            ))
        })
    }

    /// 在遮罩上 `area`（虚拟桌面物理像素）处显示画布，内容是 `image`（尺寸必须等于 `area`）；
    /// 替换已有的画布。
    ///
    /// 画布盖在背景之上、场景之下，像素由 `core` 提供：标注时显示 `core` 合成好的截图，
    /// 屏幕上看到的因此就是最终产物。在下一次 [`render`](Self::render) 时显示；
    /// [`close`](Self::close) 时释放。
    fn show_canvas(&mut self, area: Rect, image: &Image) -> Result<(), PlatformError>;

    /// 用 `patch` 替换画布的一部分，`patch` 的左上角位于虚拟桌面 `at`，必须整个落在画布内。
    /// 在下一次 [`render`](Self::render) 时显示。
    fn update_canvas(&mut self, at: Point, patch: &Image) -> Result<(), PlatformError>;

    /// 用平台的字体把 `text` 排版并栅格化成覆盖率蒙版：字号 `font_size`（物理像素），
    /// 灰度抗锯齿，遇到换行符换行、不自动换行。标注里的文字用它：遮罩画布上和最终截图里的
    /// 文字都来自同一个蒙版（ADR 0006）。遮罩打开期间可用。
    fn rasterize_text(&self, text: &str, font_size: u32) -> Result<TextMask, PlatformError>;

    /// 让 `area`（虚拟桌面物理像素）“透过”遮罩：遮罩不再盖住这块范围，用户看到的是底下
    /// 实时变化的桌面，鼠标的点击、滚轮也交给底下的窗口；`None` 恢复整块遮罩。
    /// 长截屏期间用户就在这块范围里滚动内容。
    ///
    /// 透过期间遮罩本身不出现在屏幕捕获里（包括 [`crate::RegionSampler`] 采到的画面）。
    /// 立即生效；[`close`](Self::close) 时一并恢复。
    fn set_pass_through(&mut self, area: Option<Rect>) -> Result<(), PlatformError>;

    /// 隐藏遮罩并释放冻结帧和画布；没有打开时什么也不做。
    fn close(&mut self);
}

/// 遮罩上要画的全部内容（背景之上）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayScene {
    /// 按顺序从下往上画。
    pub items: Vec<OverlayItem>,
    /// 鼠标指针在遮罩上的形状。
    pub cursor: OverlayCursor,
    /// 正在输入文字时插入点的位置（虚拟桌面物理像素）。`Some` 时遮罩接受文字输入：平台打开
    /// 输入法、把输入法的组字和候选窗口放在插入点旁边，输入的文字以 [`OverlayInput::Character`]
    /// 递交；`None` 时关闭输入法。插入点本身由 `core` 画在 `items` 里。
    pub text_caret: Option<Rect>,
}

impl OverlayScene {
    /// 场景中的全部文字标签，按绘制顺序排列。
    pub fn labels(&self) -> impl Iterator<Item = &Label> {
        self.items.iter().filter_map(|item| match item {
            OverlayItem::Label(label) => Some(label),
            _ => None,
        })
    }

    /// 场景中内容为 `text` 的 [`OverlayItem::Text`] 所在的矩形（有多个时取最上面的）。
    pub fn text_rect(&self, text: &str) -> Option<Rect> {
        self.items.iter().rev().find_map(|item| match item {
            OverlayItem::Text { rect, text: t, .. } if t == text => Some(*rect),
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
    /// 在 `rect` 内水平、垂直都居中的单行文字，没有底色（例如工具条按钮上的文字）。
    /// 超出 `rect` 的部分可能被裁掉，由调用方留足宽度。
    Text {
        rect: Rect,
        text: String,
        /// 字号（物理像素），已按所在显示器的缩放比例换算。
        font_size: f32,
        color: Color,
    },
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
    /// 左右调整大小（↔）。
    ResizeEw,
    /// 上下调整大小（↕）。
    ResizeNs,
    /// 沿左上—右下对角线调整大小（↖↘）。
    ResizeNwse,
    /// 沿右上—左下对角线调整大小（↗↙）。
    ResizeNesw,
    /// 移动（四向箭头）。
    Move,
    /// 文字输入（I 形）。
    IBeam,
}

/// 用户在遮罩上的输入，坐标为虚拟桌面物理像素。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayInput {
    /// 指针移动。修饰键按下或松开而指针没动时，平台也在原位置递交一次，`modifiers` 是新的状态。
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
    /// 输入了一个字符：键盘直接输入的或输入法确认的文字，逐字递交。只递交可打印字符；
    /// 回车、退格等以 [`OverlayInput::KeyPressed`] 递交。
    Character(char),
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
    /// 方向键（按住时随系统的按键重复多次递交）。
    Left,
    Right,
    Up,
    Down,
    /// 字母键 C（与大小写、输入法无关）。
    C,
    Backspace,
}
