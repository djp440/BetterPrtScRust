//! `bps` CLI 的平台无关部分：解析参数 → 转交给常驻实例 → 把回复换算成退出状态和输出。

use bps_platform::ResidentChannel;

use crate::request::{MODE_NAMES, mode_from_name};
use crate::{Config, Locale, Reply, Request, Texts, WhenNotRunning, forward};

/// `bps` 的退出状态。退出码是对脚本公开的接口，见 [`exit_code`](Self::exit_code)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliStatus {
    /// 常驻实例执行完了命令。
    Success,
    /// 常驻实例执行命令失败。
    Failed,
    /// 参数有误。
    Usage,
    /// 无法连接或拉起常驻实例。
    Unreachable,
    /// 用户取消了这次捕获（例如在选区遮罩上按了 Esc），没有任何产物。
    Cancelled,
}

impl CliStatus {
    pub fn exit_code(self) -> u8 {
        match self {
            CliStatus::Success => 0,
            CliStatus::Failed => 1,
            CliStatus::Usage => 2,
            CliStatus::Unreachable => 4,
            CliStatus::Cancelled => 5,
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

/// `bps` 用的界面语言：与常驻实例一致，由配置文件里的 `language` 和系统语言决定。
/// 配置文件不存在或有错误时（常驻实例此时用默认配置）按系统语言。
pub fn cli_locale(config_text: Option<&str>, system_languages: &[String]) -> Locale {
    let language = config_text
        .and_then(|text| Config::parse(text).ok())
        .map(|config| config.language)
        .unwrap_or_default();
    language.resolve(system_languages)
}

/// 执行一次 `bps` 调用，输出文本用 `locale` 的语言。`args` 不含程序名。
///
/// 命令转交给常驻实例执行；常驻实例没在运行时先拉起它。
pub fn run_cli<I, S>(args: I, channel: &mut dyn ResidentChannel, locale: Locale) -> CliOutcome
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let texts = Texts::of(locale);
    let args: Vec<S> = args.into_iter().collect();
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let request = match parse(&args, texts) {
        Parsed::Request(request) => request,
        Parsed::Help => {
            return CliOutcome {
                stdout: usage(texts),
                ..CliOutcome::new(CliStatus::Success)
            };
        }
        Parsed::Invalid(problem) => {
            return CliOutcome::with_stderr(
                CliStatus::Usage,
                format!("{problem}\n\n{}", usage(texts)),
            );
        }
    };
    match forward(&request, channel, WhenNotRunning::Launch) {
        Ok(Reply::Done | Reply::Status(_)) => CliOutcome::new(CliStatus::Success),
        Ok(Reply::Cancelled) => CliOutcome::with_stderr(
            CliStatus::Cancelled,
            format!("{}\n", texts.get("cli-cancelled")),
        ),
        Ok(Reply::Failed(message)) => CliOutcome::with_stderr(
            CliStatus::Failed,
            format!("{}\n", texts.format("cli-failed", &[("message", &message)])),
        ),
        Err(e) => CliOutcome::with_stderr(
            CliStatus::Unreachable,
            format!("{}\n", texts.format("cli-unreachable", &[("error", &e)])),
        ),
    }
}

enum Parsed {
    Request(Request),
    Help,
    /// 参数有误，附带给人看的说明。
    Invalid(String),
}

fn parse(args: &[&str], texts: &Texts) -> Parsed {
    match args {
        ["capture", mode] => match mode_from_name(mode) {
            Some(mode) => Parsed::Request(Request::Capture(mode)),
            None => Parsed::Invalid(texts.format("cli-unknown-mode", &[("mode", mode)])),
        },
        ["capture"] => Parsed::Invalid(texts.get("cli-missing-mode")),
        ["capture", _, extra, ..] => {
            Parsed::Invalid(texts.format("cli-extra-argument", &[("argument", extra)]))
        }
        ["help" | "--help" | "-h" | "/?"] => Parsed::Help,
        [] => Parsed::Invalid(texts.get("cli-missing-command")),
        [command, ..] => {
            Parsed::Invalid(texts.format("cli-unknown-command", &[("command", command)]))
        }
    }
}

fn usage(texts: &Texts) -> String {
    let mut text = texts.get("cli-usage");
    text.push_str("\n\n");
    text.push_str(&texts.get("cli-usage-modes"));
    text.push('\n');
    for (mode, name) in MODE_NAMES {
        text.push_str(&format!("  {name:<16}{}\n", mode.localized_label(texts)));
    }
    text.push('\n');
    text.push_str(&texts.get("cli-exit-codes"));
    text.push('\n');
    for (status, message) in [
        (CliStatus::Success, "cli-exit-success"),
        (CliStatus::Failed, "cli-exit-failed"),
        (CliStatus::Usage, "cli-exit-usage"),
        (CliStatus::Unreachable, "cli-exit-unreachable"),
        (CliStatus::Cancelled, "cli-exit-cancelled"),
    ] {
        text.push_str(&format!(
            "  {}  {}\n",
            status.exit_code(),
            texts.get(message)
        ));
    }
    text
}
