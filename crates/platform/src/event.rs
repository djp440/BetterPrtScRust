use crate::{
    HotkeyId, IncomingRequest, LatestRelease, OverlayInput, PlatformError, TrayMenuItemId,
};

/// 平台事件循环递交给常驻实例的触发事件。
///
/// 事件本身不含业务含义，由 `core` 把它转换成命令再执行。
/// 请求事件带着回复通道，所以事件不能复制。
#[derive(Debug)]
pub enum PlatformEvent {
    /// 用 [`crate::GlobalHotkeys::register`] 注册的热键被按下。
    HotkeyPressed(HotkeyId),
    /// 用户或系统要求常驻实例退出，例如点击了托盘菜单的“退出”。
    ExitRequested,
    /// 用 [`crate::ConfigFile::watch`] 监视的配置文件被外部修改、替换或删除。
    ConfigFileChanged,
    /// 用 [`crate::SystemScreenshotTool::watch`] 监视的“PrtSc 键交给系统截图工具”这项系统设置
    /// 可能被修改了（也可能只是同一组设置里的其他项变了）。
    PrintScreenKeySettingChanged,
    /// 其他进程（`bps` CLI、再次启动的程序、设置界面）经 [`crate::ResidentChannel`]
    /// 转交来一条请求。
    RequestReceived(IncomingRequest),
    /// 用户在选区遮罩或录屏控制条（[`crate::RecordingToolbar`]）上的输入；只在它们显示期间递交。
    OverlayInput(OverlayInput),
    /// 用 [`crate::Ticker::start`] 开始的节拍到了。
    Tick,
    /// [`crate::RegionSampler`] 采到了新的画面，用 [`crate::RegionSampler::take_sample`] 取出。
    RegionSampled,
    /// 用户选中了托盘菜单（[`crate::TrayMenu`]）里的一项。
    TrayMenuItemSelected(TrayMenuItemId),
    /// [`crate::UpdateService::request_latest_release`] 的查询结果；
    /// `Ok(None)` 表示仓库还没有发布过正式版本。
    LatestReleaseFetched(Result<Option<LatestRelease>, PlatformError>),
}
