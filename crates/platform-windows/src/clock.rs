//! 时钟：系统的本地时间与单调时间。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bps_platform::{Clock, LocalDateTime};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// 用 `GetLocalTime` 读取本地时间（已按系统时区和夏令时换算）；单调时间基于
/// `QueryPerformanceCounter`（`Instant`），从本进程第一次读取时开始计。
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

    fn monotonic(&self) -> Duration {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed()
    }
}
