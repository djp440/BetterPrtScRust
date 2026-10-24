//! 剪贴板：把 `core` 编码好的各格式字节原样放进系统剪贴板。

use std::borrow::Cow;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use bps_platform::{Clipboard, ClipboardItem, PlatformError};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows::core::{PCWSTR, w};

use crate::EventLoop;
use crate::error::win_error;

/// 其他程序正占用剪贴板时的重试次数与间隔。
const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Windows 剪贴板。
///
/// 编码好的图像写入注册格式：PNG → `"PNG"`，JPEG → `"JFIF"`，WebP → `"image/webp"`，
/// AVIF → `"image/avif"`；DIB 写入 `CF_DIB`，文件写入 `CF_HDROP`，文本写入 `CF_UNICODETEXT`。
pub struct WindowsClipboard {
    owner: Option<HWND>,
}

impl WindowsClipboard {
    /// `owner` 为常驻实例的事件循环时，剪贴板所有者是它的隐藏窗口。
    pub fn new(owner: Option<&EventLoop>) -> Self {
        Self {
            owner: owner.map(EventLoop::hwnd),
        }
    }
}

impl Clipboard for WindowsClipboard {
    fn write(&mut self, items: Vec<ClipboardItem>) -> Result<(), PlatformError> {
        let _open = OpenedClipboard::open(self.owner)?;
        unsafe { EmptyClipboard() }.map_err(|e| win_error("清空剪贴板失败", e))?;
        for item in &items {
            let registered = |name: PCWSTR| unsafe { RegisterClipboardFormatW(name) };
            let (format, bytes) = match item {
                ClipboardItem::Png(bytes) => (registered(w!("PNG")), Cow::Borrowed(bytes)),
                ClipboardItem::Jpeg(bytes) => (registered(w!("JFIF")), Cow::Borrowed(bytes)),
                ClipboardItem::WebP(bytes) => (registered(w!("image/webp")), Cow::Borrowed(bytes)),
                ClipboardItem::Avif(bytes) => (registered(w!("image/avif")), Cow::Borrowed(bytes)),
                ClipboardItem::Dib(bytes) => (u32::from(CF_DIB.0), Cow::Borrowed(bytes)),
                ClipboardItem::Files(paths) => {
                    (u32::from(CF_HDROP.0), Cow::Owned(drop_files(paths)))
                }
                ClipboardItem::Text(text) => {
                    (u32::from(CF_UNICODETEXT.0), Cow::Owned(unicode_text(text)))
                }
            };
            if format == 0 {
                return Err(PlatformError::new(format!(
                    "注册 {} 剪贴板格式失败",
                    item.format_name()
                )));
            }
            set_data(format, &bytes).map_err(|e| {
                PlatformError::new(format!("写入剪贴板 {} 格式失败：{e}", item.format_name()))
            })?;
        }
        Ok(())
    }
}

/// `CF_HDROP` 的内容：`DROPFILES` 头（20 字节，宽字符）后接各个以 0 结尾的路径，
/// 最后再加一个 0。
fn drop_files(paths: &[PathBuf]) -> Vec<u8> {
    const DROPFILES_SIZE: u32 = 20;
    let mut out = Vec::new();
    out.extend_from_slice(&DROPFILES_SIZE.to_le_bytes()); // pFiles：路径列表的偏移
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    out.extend_from_slice(&0i32.to_le_bytes()); // fNC
    out.extend_from_slice(&1i32.to_le_bytes()); // fWide：路径是 UTF-16
    for path in paths {
        for unit in path.as_os_str().encode_wide().chain([0]) {
            out.extend_from_slice(&unit.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// `CF_UNICODETEXT` 的内容：以 0 结尾的 UTF-16（系统会据此自动提供 ANSI 等其他文本格式）。
fn unicode_text(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// 已打开的剪贴板；离开作用域时关闭。
struct OpenedClipboard;

impl OpenedClipboard {
    fn open(owner: Option<HWND>) -> Result<Self, PlatformError> {
        let mut last_error = None;
        for _ in 0..OPEN_ATTEMPTS {
            match unsafe { OpenClipboard(owner) } {
                Ok(()) => return Ok(Self),
                Err(e) => last_error = Some(e),
            }
            thread::sleep(OPEN_RETRY_DELAY);
        }
        Err(match last_error {
            Some(e) => win_error("剪贴板被其他程序占用", e),
            None => PlatformError::new("剪贴板被其他程序占用"),
        })
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

/// 把 `bytes` 复制进一块可移动全局内存并交给剪贴板；成功后内存归剪贴板所有。
fn set_data(format: u32, bytes: &[u8]) -> Result<(), PlatformError> {
    unsafe {
        let memory =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| win_error("分配内存失败", e))?;
        let target = GlobalLock(memory) as *mut u8;
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(PlatformError::new("锁定内存失败"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        // 解锁到计数为 0 时返回“失败”且错误码为 0，属正常情况。
        let _ = GlobalUnlock(memory);
        if let Err(e) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(win_error("SetClipboardData 失败", e));
        }
        Ok(())
    }
}
