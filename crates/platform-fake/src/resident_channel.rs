use std::sync::{Arc, Mutex};

use bps_platform::{ChannelError, IncomingRequest, PlatformError, PlatformEvent, ResidentChannel};

/// 假的 CLI 侧通道：把请求包装成 [`PlatformEvent::RequestReceived`]，同步交给测试提供的
/// “常驻实例”（通常是 `|event| { instance.handle(event); }`），再取回它的回复。
///
/// 这样 CLI 一侧的转交逻辑和常驻实例一侧的请求处理在同一个测试里首尾相接，
/// 中间经过的正是真实平台也会递交的事件。
pub struct FakeResidentChannel {
    resident: Box<dyn FnMut(PlatformEvent)>,
    running: bool,
    launch_fails: bool,
    launches: usize,
}

impl FakeResidentChannel {
    /// 常驻实例已在运行，收到的请求立即交给 `resident`。
    pub fn running(resident: impl FnMut(PlatformEvent) + 'static) -> Self {
        Self {
            resident: Box::new(resident),
            running: true,
            launch_fails: false,
            launches: 0,
        }
    }

    /// 常驻实例没在运行：被拉起之前连不上，拉起之后请求交给 `resident`。
    pub fn not_running(resident: impl FnMut(PlatformEvent) + 'static) -> Self {
        Self {
            running: false,
            ..Self::running(resident)
        }
    }

    /// 让拉起常驻实例失败（例如找不到程序文件）。
    pub fn launch_fails(mut self) -> Self {
        self.launch_fails = true;
        self
    }

    /// 常驻实例被拉起的次数。
    pub fn launches(&self) -> usize {
        self.launches
    }
}

impl ResidentChannel for FakeResidentChannel {
    fn send(&mut self, message: &str) -> Result<String, ChannelError> {
        if !self.running {
            return Err(ChannelError::NotRunning);
        }
        let reply = Arc::new(Mutex::new(None));
        let slot = reply.clone();
        let request = IncomingRequest::new(message, move |response| {
            *slot.lock().unwrap() = Some(response);
        });
        (self.resident)(PlatformEvent::RequestReceived(request));
        let response = reply.lock().unwrap().take();
        response.ok_or_else(|| ChannelError::Failed(PlatformError::new("常驻实例没有回复")))
    }

    fn launch_resident_instance(&mut self) -> Result<(), PlatformError> {
        self.launches += 1;
        if self.launch_fails {
            return Err(PlatformError::new("找不到常驻实例的程序文件"));
        }
        self.running = true;
        Ok(())
    }
}
