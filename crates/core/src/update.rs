//! 检查更新（ADR 0004）：比较 GitHub Releases 上最新正式版本的 tag 与当前版本。

use std::cmp::Ordering;

/// 当前程序的版本（工作区 `Cargo.toml` 的 `version`）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 检查更新时查询的 GitHub 仓库。
pub(crate) const RELEASES_REPOSITORY: &str = "djp440/BetterPrtScRust";

/// `tag`（如 `v0.2.0`）是否比 `current` 新；`tag` 不是可识别的版本号时返回 `None`。
pub(crate) fn is_newer(tag: &str, current: &str) -> Option<bool> {
    let latest = Version::parse(tag)?;
    let current = Version::parse(current)?;
    Some(latest > current)
}

/// 语义化版本号：`主.次.修订`，可带 `-预发布` 后缀；`+构建信息` 忽略。
#[derive(Debug, PartialEq, Eq)]
struct Version {
    numbers: [u64; 3],
    prerelease: Option<String>,
}

impl Version {
    fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text
            .strip_prefix('v')
            .or_else(|| text.strip_prefix('V'))
            .unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(version, _)| version);
        let (core, prerelease) = match text.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_owned())),
            Some(_) => return None,
            None => (text, None),
        };
        let mut numbers = [0u64; 3];
        let parts: Vec<&str> = core.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        for (slot, part) in numbers.iter_mut().zip(&parts) {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        Some(Version {
            numbers,
            prerelease,
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    /// 先比数字；数字相同时，正式版比预发布版新，预发布版之间按字符串比较。
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers
            .cmp(&other.numbers)
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}
