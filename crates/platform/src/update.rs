/// GitHub Releases 上最新的正式版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestRelease {
    /// 版本的 tag，例如 `v0.2.0`。
    pub tag: String,
    /// 这个版本的发布页面，供用户自行下载。
    pub page_url: String,
}

/// 检查更新：查询 GitHub Releases 上最新的正式版本（ADR 0004 允许的两种联网行为之一）。
///
/// 只查询、不下载。只有用户开启了“检查更新”时 `core` 才会调用它；关闭时程序不发任何
/// 网络请求。查询在后台进行，结果经平台事件循环以
/// [`crate::PlatformEvent::LatestReleaseFetched`] 递交，不阻塞热键。
pub trait UpdateService {
    /// 开始查询 GitHub 仓库 `repository`（形如 `owner/name`）的最新正式版本。
    fn request_latest_release(&mut self, repository: &str);
}
