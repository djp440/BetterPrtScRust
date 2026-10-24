//! `bps` CLI 的平台无关部分：解析参数 → 转交给常驻实例 → 把回复换算成退出状态和输出。

use bps_platform::ResidentChannel;

use crate::request::{MODE_NAMES, mode_from_name};
use crate::{Reply, Request, WhenNotRunning, forward};

/// `bps` 的退出状态。退出码是对脚本公开的接口，见 [`exit_code`](Self::exit_code)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliStatus {
    /// 常驻实例执行完了命令。
    Success,
    /// 常驻实例执行命令失败。
    Failed,
    /// 参数有误。
    Usage,
    /// 常驻实例暂不支持这条命令（例如尚未实现的捕获模式）。
    Unsupported,
    /// 无法连接或拉起常驻实例。
    Unreachable,
}

impl CliStatus {
    pub fn exit_code(self) -> u8 {
        match self {
            CliStatus::Success => 0,
            CliStatus::Failed => 1,
            CliStatus::Usage => 2,
            CliStatus::Unsupported => 3,
            CliStatus::Unreachable => 4,
        }
    }
}

/// 一次 `bps` 调用的结果：退出状态，以及要写到标准输出 / 标准错误的文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutcome {
    pub status: CliStatus,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutcome {
    fn new(status: CliStatus) -> Self {
        Self {
            status,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn with_stderr(status: CliStatus, message: impl Into<String>) -> Self {
        Self {
            stderr: message.into(),
            ..Self::new(status)
        }
    }
}

/// 执行一次 `bps` 调用。`args` 不含程序名。
///
/// 命令转交给常驻实例执行；常驻实例没在运行时先拉起它。
pub fn run_cli<I, S>(args: I, channel: &mut dyn ResidentChannel) -> CliOutcome
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<S> = args.into_iter().collect();
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let request = match parse(&args) {
        Parsed::Request(request) => request,
        Parsed::Help => {
            return CliOutcome {
                stdout: usage(),
                ..CliOutcome::new(CliStatus::Success)
            };
        }
        Parsed::Invalid(problem) => {
            return CliOutcome::with_stderr(CliStatus::Usage, format!("{problem}\n\n{}", usage()));
        }
    };
    match forward(&request, channel, WhenNotRunning::Launch) {
        Ok(Reply::Done) => CliOutcome::new(CliStatus::Success),
        Ok(Reply::Unsupported(message)) => {
            CliOutcome::with_stderr(CliStatus::Unsupported, format!("{message}\n"))
        }
        Ok(Reply::Failed(message)) => {
            CliOutcome::with_stderr(CliStatus::Failed, format!("执行失败：{message}\n"))
        }
        Err(e) => CliOutcome::with_stderr(
            CliStatus::Unreachable,
            format!("无法把命令转交给常驻实例：{e}\n"),
        ),
    }
}

enum Parsed {
    Request(Request),
    Help,
    /// 参数有误，附带给人看的说明。
    Invalid(String),
}

fn parse(args: &[&str]) -> Parsed {
    match args {
        ["capture", mode] => match mode_from_name(mode) {
            Some(mode) => Parsed::Request(Request::Capture(mode)),
            None => Parsed::Invalid(format!("未知的捕获模式：{mode}")),
        },
        ["capture"] => Parsed::Invalid("缺少捕获模式".to_owned()),
        ["capture", _, extra, ..] => Parsed::Invalid(format!("多余的参数：{extra}")),
        ["help" | "--help" | "-h" | "/?"] => Parsed::Help,
        [] => Parsed::Invalid("缺少命令".to_owned()),
        [command, ..] => Parsed::Invalid(format!("未知的命令：{command}")),
    }
}

fn usage() -> String {
    let mut text = String::from(
        "用法：bps capture <捕获模式>\n\
         \n\
         把捕获命令转交给 BetterPrtSc 常驻实例执行，行为与按下对应热键一致：\n\
         采用第一条该捕获模式的热键绑定的截后动作，没有这样的绑定时复制到剪贴板。\n\
         常驻实例没在运行时会先启动它。\n\
         \n\
         捕获模式：\n",
    );
    for (mode, name) in MODE_NAMES {
        text.push_str(&format!("  {name:<16}{}\n", mode.label()));
    }
    text.push_str(
        "\n\
         退出码：\n\
         \x20 0  成功\n\
         \x20 1  常驻实例执行命令失败\n\
         \x20 2  参数有误\n\
         \x20 3  常驻实例暂不支持该捕获模式\n\
         \x20 4  无法连接或启动常驻实例\n",
    );
    text
}
