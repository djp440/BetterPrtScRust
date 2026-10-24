/// 托盘菜单里的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub id: TrayMenuItemId,
    /// 显示给用户的文字（已按界面语言本地化）。
    pub label: String,
}

/// 托盘菜单项的标识，由 `core` 分配；用户选中某项时随 [`crate::PlatformEvent`] 返回。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrayMenuItemId(pub u32);

/// 托盘图标的右键菜单。
///
/// 菜单的内容和文字都由 `core` 决定（界面语言切换时会重新设置）；平台只负责显示，
/// 用户选中某一项后，平台事件循环递交 [`crate::PlatformEvent::TrayMenuItemSelected`]。
pub trait TrayMenu {
    /// 用 `items` 替换整个菜单，按顺序显示。
    fn set_items(&mut self, items: Vec<TrayMenuItem>);
}
