//! 时钟：系统的本地时间。

use bps_platform::{Clock, LocalDateTime};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// 用 `GetLocalTime` 读取本地时间（已按系统时区和夏令时换算）。
#[derive(Debug, Default)]
pub struct WindowsClock;

impl WindowsClock {
    pub fn new() -> Self {
        Self
    }
}

impl Clock for WindowsClock {
    fn now(&self) -> LocalDateTime {
        let t = unsafe { GetLocalTime() };
        LocalDateTime {
            year: t.wYear,
            month: t.wMonth as u8,
            day: t.wDay as u8,
            hour: t.wHour as u8,
            minute: t.wMinute as u8,
            second: t.wSecond as u8,
            millisecond: t.wMilliseconds,
        }
    }
}
