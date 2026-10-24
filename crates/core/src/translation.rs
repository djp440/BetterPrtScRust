//! 翻译：把结果面板里的文字交给用户自己配置的翻译服务（OpenAI 兼容接口或 DeepL）。
//!
//! ADR 0004：只有用户主动点“翻译”时才联网，并且请求里只有文字，截图本身绝不上传。
//! 请求的构造和回复的解析都在这里（平台无关），平台只负责 [`HttpTransport`] 发送。
//! API Key 不得出现在日志、错误提示和调试输出里。

use std::fmt;

use bps_platform::{HttpRequest, HttpTransport};
use serde_json::{Value, json};

use crate::{Locale, Texts};

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const OPENAI_DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEEPL_FREE_URL: &str = "https://api-free.deepl.com/v2/translate";
const DEEPL_PRO_URL: &str = "https://api.deepl.com/v2/translate";
/// 错误提示里引用的服务端说明的最大字符数。
const MAX_DETAIL_CHARS: usize = 200;

/// 翻译服务的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationProvider {
    /// OpenAI 兼容的 `chat/completions` 接口。
    OpenAi,
    DeepL,
}

impl TranslationProvider {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "deepl" => Some(Self::DeepL),
            _ => None,
        }
    }
}

/// 翻译成哪种语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetLanguage {
    /// 跟随界面语言。
    #[default]
    Auto,
    ZhCn,
    En,
    Ja,
    Ko,
    Fr,
    De,
    Es,
    Ru,
}

impl TargetLanguage {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "zh-cn" | "zh" => Self::ZhCn,
            "en" => Self::En,
            "ja" => Self::Ja,
            "ko" => Self::Ko,
            "fr" => Self::Fr,
            "de" => Self::De,
            "es" => Self::Es,
            "ru" => Self::Ru,
            _ => return None,
        })
    }

    fn resolve(self, interface: Locale) -> Self {
        match (self, interface) {
            (Self::Auto, Locale::ZhCn) => Self::ZhCn,
            (Self::Auto, Locale::En) => Self::En,
            (other, _) => other,
        }
    }

    fn english_name(self) -> &'static str {
        match self {
            Self::Auto | Self::ZhCn => "Simplified Chinese",
            Self::En => "English",
            Self::Ja => "Japanese",
            Self::Ko => "Korean",
            Self::Fr => "French",
            Self::De => "German",
            Self::Es => "Spanish",
            Self::Ru => "Russian",
        }
    }

    fn deepl_code(self) -> &'static str {
        match self {
            Self::Auto | Self::ZhCn => "ZH",
            Self::En => "EN-US",
            Self::Ja => "JA",
            Self::Ko => "KO",
            Self::Fr => "FR",
            Self::De => "DE",
            Self::Es => "ES",
            Self::Ru => "RU",
        }
    }
}

/// 配置文件 `[translation]` 里的翻译服务设置。
#[derive(Clone, PartialEq, Eq)]
pub struct TranslationConfig {
    pub provider: TranslationProvider,
    /// 用户自己的 API Key。
    pub api_key: String,
    /// 接口地址：OpenAI 兼容接口的基地址（缺省官方地址），DeepL 的完整请求地址（缺省按密钥选）。
    pub base_url: Option<String>,
    /// OpenAI 兼容接口用的模型（缺省 `gpt-4o-mini`）。
    pub model: Option<String>,
    pub target_language: TargetLanguage,
}

impl fmt::Debug for TranslationConfig {
    /// 不输出 API Key。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TranslationConfig")
            .field("provider", &self.provider)
            .field("api_key", &"<hidden>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("target_language", &self.target_language)
            .finish()
    }
}

/// 翻译失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationError {
    /// 连不上服务（网络问题、超时等）。
    Network(String),
    /// 服务拒绝了 API Key（HTTP 401、403）。
    Rejected(u16),
    /// 服务返回了其他错误状态；说明可能为空。
    Http { status: u16, detail: String },
    /// 回复无法理解。
    UnreadableReply,
}

impl TranslationError {
    /// 用 `texts` 的语言说明失败原因。
    pub fn localized(&self, texts: &Texts) -> String {
        match self {
            Self::Network(detail) => {
                texts.format("translation-error-network", &[("detail", detail)])
            }
            Self::Rejected(status) => {
                texts.format("translation-error-rejected", &[("status", status)])
            }
            Self::Http { status, detail } if detail.is_empty() => {
                texts.format("translation-error-http", &[("status", status)])
            }
            Self::Http { status, detail } => texts.format(
                "translation-error-http-detail",
                &[("status", status), ("detail", detail)],
            ),
            Self::UnreadableReply => texts.get("translation-error-reply"),
        }
    }
}

impl fmt::Display for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized(Texts::of(Locale::ZhCn)))
    }
}

impl std::error::Error for TranslationError {}

/// 翻译 `text`：构造请求、发送、解析回复。
pub(crate) fn translate(
    config: &TranslationConfig,
    interface: Locale,
    text: &str,
    transport: &dyn HttpTransport,
) -> Result<String, TranslationError> {
    let target = config.target_language.resolve(interface);
    let request = build_request(config, target, text);
    let response = transport
        .post(&request)
        .map_err(|e| TranslationError::Network(redact(e.message(), &config.api_key)))?;
    match response.status {
        200..=299 => parse_reply(config.provider, &response.body),
        401 | 403 => Err(TranslationError::Rejected(response.status)),
        status => Err(TranslationError::Http {
            status,
            detail: error_detail(&response.body, &config.api_key),
        }),
    }
}

fn build_request(config: &TranslationConfig, target: TargetLanguage, text: &str) -> HttpRequest {
    let json_header = ("Content-Type".to_owned(), "application/json".to_owned());
    match config.provider {
        TranslationProvider::OpenAi => {
            let base = config.base_url.as_deref().unwrap_or(OPENAI_BASE_URL);
            let instruction = format!(
                "You are a translation engine. Translate the text from the user into {}. \
                 Output only the translation, without explanations or quotation marks.",
                target.english_name()
            );
            let body = json!({
                "model": config.model.as_deref().unwrap_or(OPENAI_DEFAULT_MODEL),
                "temperature": 0,
                "messages": [
                    { "role": "system", "content": instruction },
                    { "role": "user", "content": text },
                ],
            });
            HttpRequest {
                url: format!("{}/chat/completions", base.trim_end_matches('/')),
                headers: vec![
                    json_header,
                    (
                        "Authorization".to_owned(),
                        format!("Bearer {}", config.api_key),
                    ),
                ],
                body: body.to_string().into_bytes(),
            }
        }
        TranslationProvider::DeepL => {
            let url = config.base_url.clone().unwrap_or_else(|| {
                if config.api_key.ends_with(":fx") {
                    DEEPL_FREE_URL.to_owned()
                } else {
                    DEEPL_PRO_URL.to_owned()
                }
            });
            let body = json!({ "text": [text], "target_lang": target.deepl_code() });
            HttpRequest {
                url,
                headers: vec![
                    json_header,
                    (
                        "Authorization".to_owned(),
                        format!("DeepL-Auth-Key {}", config.api_key),
                    ),
                ],
                body: body.to_string().into_bytes(),
            }
        }
    }
}

fn parse_reply(provider: TranslationProvider, body: &[u8]) -> Result<String, TranslationError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| TranslationError::UnreadableReply)?;
    let text = match provider {
        TranslationProvider::OpenAi => value.pointer("/choices/0/message/content"),
        TranslationProvider::DeepL => value.pointer("/translations/0/text"),
    };
    text.and_then(Value::as_str)
        .map(|t| t.trim().to_owned())
        .ok_or(TranslationError::UnreadableReply)
}

/// 服务端错误回复里的说明（已去掉 API Key、截短）。
fn error_detail(body: &[u8], api_key: &str) -> String {
    let text = String::from_utf8_lossy(body);
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| text.into_owned());
    let message = redact(message.trim(), api_key);
    message.chars().take(MAX_DETAIL_CHARS).collect()
}

/// 去掉 `text` 里的 API Key。
fn redact(text: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        text.to_owned()
    } else {
        text.replace(api_key, "***")
    }
}
