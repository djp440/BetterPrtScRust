//! 检查更新：用 WinHTTP 查询 GitHub Releases 上最新的正式版本（ADR 0004）。
//!
//! 只发一个 GET 请求读取版本号和发布页面，不下载任何文件。请求在后台线程进行，
//! 结果经 [`EventSender`] 交回事件循环。只有 `core` 在用户开启“检查更新”时才会调用。

use std::thread;

use bps_platform::{LatestRelease, PlatformError, PlatformEvent, UpdateService};
use serde::Deserialize;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::EventSender;
use crate::error::win_error;

const API_HOST: PCWSTR = w!("api.github.com");
/// 解析、连接、发送、接收各自的超时。
const TIMEOUT_MS: i32 = 15_000;
/// 回复正文的上限；最新版本的信息只有几 KB 到几十 KB。
const MAX_BODY_BYTES: usize = 1 << 20;

/// 查询 GitHub Releases 的检查更新服务。
pub struct GitHubReleases {
    events: EventSender,
}

impl GitHubReleases {
    /// 查询结果以 [`PlatformEvent::LatestReleaseFetched`] 投递给 `events` 所属的事件循环。
    pub fn new(events: EventSender) -> Self {
        Self { events }
    }
}

impl UpdateService for GitHubReleases {
    fn request_latest_release(&mut self, repository: &str) {
        let events = self.events.clone();
        let repository = repository.to_owned();
        let spawned = thread::Builder::new()
            .name("bps-update-check".to_owned())
            .spawn(move || {
                let result = latest_release(&repository);
                let _ = events.send(PlatformEvent::LatestReleaseFetched(result));
            });
        if let Err(e) = spawned {
            let error = PlatformError::new(format!("无法启动检查更新线程：{e}"));
            let _ = self
                .events
                .send(PlatformEvent::LatestReleaseFetched(Err(error)));
        }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// 读取 `repository`（`owner/name`）最新的正式版本；还没有发布过时返回 `Ok(None)`。
pub fn latest_release(repository: &str) -> Result<Option<LatestRelease>, PlatformError> {
    let path = format!("/repos/{repository}/releases/latest");
    let (status, body) = https_get(API_HOST, &path)?;
    match status {
        200 => {
            let release: Release = serde_json::from_slice(&body)
                .map_err(|e| PlatformError::new(format!("无法解析 GitHub 的回复：{e}")))?;
            Ok(Some(LatestRelease {
                tag: release.tag_name,
                page_url: release.html_url,
            }))
        }
        404 => Ok(None),
        other => Err(PlatformError::new(format!(
            "GitHub 返回了 HTTP {other}（可能是请求过于频繁，稍后会再试）"
        ))),
    }
}

/// WinHTTP 句柄，离开作用域时关闭。
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(raw: *mut core::ffi::c_void, what: &str) -> Result<Self, PlatformError> {
        if raw.is_null() {
            Err(win_error(what, windows::core::Error::from_thread()))
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// 发一个 HTTPS GET 请求，返回状态码和正文。
fn https_get(host: PCWSTR, path: &str) -> Result<(u32, Vec<u8>), PlatformError> {
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
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)
            .map_err(|e| win_error("设置超时失败", e))?;
        let connection = Handle::new(
            WinHttpConnect(session.0, host, INTERNET_DEFAULT_HTTPS_PORT, 0),
            "连接 GitHub 失败",
        )?;
        let path = HSTRING::from(path);
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                &path,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "创建请求失败",
        )?;
        let headers: Vec<u16> =
            "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n"
                .encode_utf16()
                .collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)
            .map_err(|e| win_error("无法连接 GitHub", e))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| win_error("没有收到 GitHub 的回复", e))?;

        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(
            request.0,
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
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .map_err(|e| win_error("读取 GitHub 的回复失败", e))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_BODY_BYTES {
                return Err(PlatformError::new("GitHub 的回复过长"));
            }
        }
        Ok((status, body))
    }
}
