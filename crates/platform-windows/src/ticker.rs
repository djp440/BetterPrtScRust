//! 节拍：高精度可等待定时器，由事件循环和其他句柄一起等待（不另开线程）。
//!
//! 定时器到点后事件循环递交 `PlatformEvent::Tick`；队列里已经有一个还没处理的节拍时不再追加，
//! 处理不及时的节拍因此会合并，收到节拍的一方按单调时间决定该做什么。

use std::rc::Rc;
use std::time::Duration;

use bps_platform::{PlatformError, Ticker};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CancelWaitableTimer, CreateWaitableTimerExW,
    SetWaitableTimer, TIMER_ALL_ACCESS,
};
use windows::core::PCWSTR;

use crate::error::win_error;
use crate::event_loop::Shared;

/// 事件循环等待的定时器句柄。
pub(crate) struct WaitableTimer(HANDLE);

impl WaitableTimer {
    /// 自动重置的定时器：一次等待成功后自动复位。优先用高精度定时器（Windows 10 1803 起），
    /// 否则退回普通定时器（精度约 15.6 ms，节拍会更不均匀，但录屏按单调时间补帧，帧率不受影响）。
    fn new() -> Result<Self, PlatformError> {
        let create = |flags| unsafe {
            CreateWaitableTimerExW(None, PCWSTR::null(), flags, TIMER_ALL_ACCESS.0)
        };
        create(CREATE_WAITABLE_TIMER_HIGH_RESOLUTION)
            .or_else(|_| create(0))
            .map(Self)
            .map_err(|e| win_error("创建节拍定时器失败", e))
    }

    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Drop for WaitableTimer {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelWaitableTimer(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// 绑定到事件循环的节拍，由 [`crate::EventLoop::ticker`] 创建，必须在事件循环的线程上使用。
pub struct WindowsTicker {
    shared: Rc<Shared>,
}

impl WindowsTicker {
    pub(crate) fn new(shared: Rc<Shared>) -> Self {
        Self { shared }
    }
}

impl Ticker for WindowsTicker {
    fn start(&mut self, period: Duration) -> Result<(), PlatformError> {
        if period.is_zero() {
            return Err(PlatformError::new("节拍周期必须大于 0"));
        }
        let mut slot = self.shared.ticker.borrow_mut();
        if slot.is_none() {
            *slot = Some(WaitableTimer::new()?);
        }
        let timer = slot.as_ref().expect("刚刚确保过");
        // 首次到点：相对时间（负数），单位 100 ns；之后的周期只能以毫秒计，向下取整，
        // 宁可略快一点（多出的节拍什么也不做），不要慢于帧率。
        let due = -((period.as_nanos() / 100).max(1) as i64);
        let period_ms = (period.as_millis() as i32).max(1);
        unsafe { SetWaitableTimer(timer.handle(), &due, period_ms, None, None, false) }
            .map_err(|e| win_error("启动节拍定时器失败", e))
    }

    fn stop(&mut self) {
        self.shared.ticker.borrow_mut().take();
    }
}
