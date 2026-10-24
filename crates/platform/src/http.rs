use std::fmt;

use crate::PlatformError;

/// 一个 HTTP POST 请求。
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// 完整的 URL（`https://…`；用户自建的兼容接口也可以是 `http://`）。
    pub url: String,
    /// 请求头（名称，值）。值里可能有 API Key。
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpRequest {
    /// 请求头的值可能是 API Key，调试输出里只列出名称。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("headers", &names)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// HTTP 回复。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// 发 HTTP POST 请求。目前只给翻译用（ADR 0004：用户主动触发翻译时只发送识别出的文字）。
///
/// 只在结果面板的子进程里使用，常驻实例不会调用它。调用会阻塞到收到回复或超时，
/// 界面应在后台线程调用。
pub trait HttpTransport {
    /// 发送 `request`，返回回复（任何状态码都算收到回复）；连不上、超时等返回错误。
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, PlatformError>;
}
