//! 用 WinHTTP 发 HTTP POST 请求（翻译，ADR 0004）。
//!
//! 只在结果面板的子进程里、用户点“翻译”之后被调用；常驻实例不会用到它。
//! 请求头里可能带 API Key：错误信息和日志里都不得包含请求头的值。

use bps_platform::{HttpRequest, HttpResponse, HttpTransport, PlatformError};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTP_PORT, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::error::win_error;
use crate::update::Handle;

/// 解析、连接、发送各自的超时。
const TIMEOUT_MS: i32 = 15_000;
/// 等待回复的超时：大模型生成译文可能较慢。
const RECEIVE_TIMEOUT_MS: i32 = 120_000;
/// 回复正文的上限。
const MAX_BODY_BYTES: usize = 4 << 20;

/// WinHTTP 实现的 [`HttpTransport`]。
pub struct WinHttpTransport;

impl HttpTransport for WinHttpTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, PlatformError> {
        let url = Url::parse(&request.url)?;
        post(&url, request)
    }
}

/// 拆开的 URL。
#[derive(Debug, PartialEq, Eq)]
struct Url {
    secure: bool,
    host: String,
    port: u16,
    /// 路径和查询，以 `/` 开头。
    path: String,
}

impl Url {
    fn parse(url: &str) -> Result<Self, PlatformError> {
        let invalid = || PlatformError::new(format!("无效的接口地址：{url}"));
        let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(invalid());
        };
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_owned()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_owned()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().map_err(|_| invalid())?),
            None if secure => (authority, INTERNET_DEFAULT_HTTPS_PORT),
            None => (authority, INTERNET_DEFAULT_HTTP_PORT),
        };
        if host.is_empty() || host.contains('@') {
            return Err(invalid());
        }
        Ok(Self {
            secure,
            host: host.to_owned(),
            port,
            path,
        })
    }
}

fn post(url: &Url, request: &HttpRequest) -> Result<HttpResponse, PlatformError> {
    let agent = HSTRING::from(format!("BetterPrtSc/{}", env!("CARGO_PKG_VERSION")));
    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                &agent,
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "初始化 WinHTTP 失败",
        )?;
        WinHttpSetTimeouts(
            session.0,
            TIMEOUT_MS,
            TIMEOUT_MS,
            TIMEOUT_MS,
            RECEIVE_TIMEOUT_MS,
        )
        .map_err(|e| win_error("设置超时失败", e))?;
        let host = HSTRING::from(url.host.as_str());
        let connection = Handle::new(
            WinHttpConnect(session.0, &host, url.port, 0),
            "连接翻译服务失败",
        )?;
        let path = HSTRING::from(url.path.as_str());
        let flags = if url.secure {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let handle = Handle::new(
            WinHttpOpenRequest(
                connection.0,
                w!("POST"),
                &path,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                flags,
            ),
            "创建请求失败",
        )?;
        let headers: Vec<u16> = request
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>()
            .encode_utf16()
            .collect();
        let length = request.body.len() as u32;
        WinHttpSendRequest(
            handle.0,
            Some(&headers),
            Some(request.body.as_ptr().cast()),
            length,
            length,
            0,
        )
        .map_err(|e| win_error("无法连接翻译服务", e))?;
        WinHttpReceiveResponse(handle.0, std::ptr::null_mut())
            .map_err(|e| win_error("没有收到翻译服务的回复", e))?;

        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(
            handle.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut size,
            &mut index,
        )
        .map_err(|e| win_error("读取 HTTP 状态失败", e))?;

        let mut body = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                handle.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .map_err(|e| win_error("读取翻译服务的回复失败", e))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_BODY_BYTES {
                return Err(PlatformError::new("翻译服务的回复过长"));
            }
        }
        Ok(HttpResponse {
            status: status as u16,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let url = Url::parse("https://api.openai.com/v1/chat/completions").unwrap();
        assert_eq!(
            url,
            Url {
                secure: true,
                host: "api.openai.com".to_owned(),
                port: 443,
                path: "/v1/chat/completions".to_owned()
            }
        );
        let url = Url::parse("http://localhost:11434/v1/x?a=b").unwrap();
        assert_eq!(
            (url.secure, url.host.as_str(), url.port),
            (false, "localhost", 11434)
        );
        assert_eq!(url.path, "/v1/x?a=b");
        assert!(Url::parse("ftp://x").is_err());
        assert!(Url::parse("https://user@host/").is_err());
    }
}
