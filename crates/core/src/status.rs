//! 常驻实例的状态：设置界面经命名管道查询（请求 `status`），用来把注册失败的热键绑定标红、
//! 显示检查更新的结果。
//!
//! 回复的文本形式：第一行 `status`，之后每行一项，字段用制表符分隔：
//! - `failure <下标> <热键> taken`
//! - `failure <下标> <热键> duplicate <与之重复的下标>`
//! - `failure <下标> <热键> platform <系统给出的说明>`
//! - `update disabled` / `update checking` / `update up-to-date`
//! - `update available <版本> <发布页面>` / `update failed <说明>`
//!
//! 说明文字里的制表符和换行在发送前换成空格。无法识别的行忽略。

use bps_platform::{Hotkey, PlatformError};

use crate::RegistrationError;

/// 常驻实例当前的状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResidentStatus {
    /// 最近一次注册热键时失败的绑定，按配置中的顺序排列。
    pub registration_failures: Vec<BindingFailure>,
    /// 检查更新的状态。
    pub update: UpdateStatus,
}

/// 一条注册失败的热键绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingFailure {
    /// 这条绑定在配置中的位置（从 0 开始）。
    pub index: usize,
    /// 这条绑定的热键，设置界面用它确认标红的是同一条绑定。
    pub hotkey: Hotkey,
    pub error: RegistrationError,
}

/// 检查更新的状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    /// 没有开启“检查更新”，不发任何网络请求。
    #[default]
    Disabled,
    /// 正在查询 GitHub Releases。
    Checking,
    /// 当前已是最新版本。
    UpToDate,
    /// 有新版本，只提示、不下载。
    Available {
        /// 新版本的 tag，例如 `v0.2.0`。
        version: String,
        /// 发布页面，用户在浏览器里自行下载。
        page_url: String,
    },
    /// 查询失败，附带给人看的说明。
    Failed(String),
}

const HEADER: &str = "status";

impl ResidentStatus {
    pub(crate) fn encode(&self) -> String {
        let mut lines = vec![HEADER.to_owned()];
        for failure in &self.registration_failures {
            let reason = match &failure.error {
                RegistrationError::TakenByOtherProgram => "taken".to_owned(),
                RegistrationError::DuplicateOf(first) => format!("duplicate\t{first}"),
                RegistrationError::Platform(e) => format!("platform\t{}", one_field(e.message())),
            };
            lines.push(format!(
                "failure\t{}\t{}\t{reason}",
                failure.index, failure.hotkey
            ));
        }
        lines.push(match &self.update {
            UpdateStatus::Disabled => "update\tdisabled".to_owned(),
            UpdateStatus::Checking => "update\tchecking".to_owned(),
            UpdateStatus::UpToDate => "update\tup-to-date".to_owned(),
            UpdateStatus::Available { version, page_url } => format!(
                "update\tavailable\t{}\t{}",
                one_field(version),
                one_field(page_url)
            ),
            UpdateStatus::Failed(message) => format!("update\tfailed\t{}", one_field(message)),
        });
        lines.join("\n")
    }

    /// 解析回复文本；不是状态回复时返回 `None`。
    pub(crate) fn decode(message: &str) -> Option<ResidentStatus> {
        let mut lines = message.split('\n');
        if lines.next()? != HEADER {
            return None;
        }
        let mut status = ResidentStatus::default();
        for line in lines {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["failure", index, hotkey, reason @ ..] => {
                    let (Ok(index), Ok(hotkey)) = (index.parse(), hotkey.parse()) else {
                        continue;
                    };
                    let error = match reason {
                        ["taken"] => RegistrationError::TakenByOtherProgram,
                        ["duplicate", first] => match first.parse() {
                            Ok(first) => RegistrationError::DuplicateOf(first),
                            Err(_) => continue,
                        },
                        ["platform", message] => {
                            RegistrationError::Platform(PlatformError::new(*message))
                        }
                        _ => continue,
                    };
                    status.registration_failures.push(BindingFailure {
                        index,
                        hotkey,
                        error,
                    });
                }
                ["update", "disabled"] => status.update = UpdateStatus::Disabled,
                ["update", "checking"] => status.update = UpdateStatus::Checking,
                ["update", "up-to-date"] => status.update = UpdateStatus::UpToDate,
                ["update", "available", version, page_url] => {
                    status.update = UpdateStatus::Available {
                        version: (*version).to_owned(),
                        page_url: (*page_url).to_owned(),
                    }
                }
                ["update", "failed", message] => {
                    status.update = UpdateStatus::Failed((*message).to_owned())
                }
                _ => {}
            }
        }
        Some(status)
    }
}

/// 把文字里的制表符和换行换成空格（连续的只换成一个），使它能放进一个字段。
fn one_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_break = false;
    for c in text.chars() {
        if matches!(c, '\t' | '\n' | '\r') {
            if !last_was_break {
                out.push(' ');
            }
            last_was_break = true;
        } else {
            out.push(c);
            last_was_break = false;
        }
    }
    out
}
