//! 剪贴板：把 `core` 编码好的各格式字节原样放进系统剪贴板。

use std::thread;
use std::time::Duration;

use bps_platform::{Clipboard, ClipboardItem, PlatformError};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIB;
use windows::core::w;

use crate::EventLoop;
use crate::error::win_error;

/// 其他程序正占用剪贴板时的重试次数与间隔。
const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Windows 剪贴板。PNG 写入注册格式 `"PNG"`，DIB 写入 `CF_DIB`。
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
            let (format, bytes) = match item {
                ClipboardItem::Png(bytes) => {
                    (unsafe { RegisterClipboardFormatW(w!("PNG")) }, bytes)
                }
                ClipboardItem::Dib(bytes) => (u32::from(CF_DIB.0), bytes),
            };
            if format == 0 {
                return Err(PlatformError::new("注册 PNG 剪贴板格式失败"));
            }
            set_data(format, bytes).map_err(|e| {
                PlatformError::new(format!("写入剪贴板 {} 格式失败：{e}", item.format_name()))
            })?;
        }
        Ok(())
    }
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
