use bps_platform::{HotkeyId, Platform, PlatformEvent};

use crate::binding::HotkeyRegistrationFailure;
use crate::{Command, CoreError, Flow, HotkeyBinding, action, capture};

/// 常驻实例的核心：持有平台能力和热键绑定，把触发事件转换成命令并执行。
///
/// 宿主（`app` 里的平台事件循环）把 [`PlatformEvent`] 交给 [`handle`](Self::handle)，
/// 在返回 [`Flow::Exit`] 时退出循环。
pub struct ResidentInstance {
    platform: Platform,
    bindings: Vec<HotkeyBinding>,
    /// 已成功注册的热键，下标与 `bindings` 对应。
    registered: Vec<HotkeyId>,
}

impl ResidentInstance {
    pub fn new(platform: Platform, bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            platform,
            bindings,
            registered: Vec::new(),
        }
    }

    /// 为每条热键绑定注册全局热键，返回注册失败的绑定；失败的绑定不影响其他绑定。
    #[must_use = "热键注册失败不能静默忽略"]
    pub fn register_hotkeys(&mut self) -> Vec<HotkeyRegistrationFailure> {
        let mut failures = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            let id = hotkey_id(index);
            if self.registered.contains(&id) {
                continue;
            }
            match self.platform.hotkeys.register(id, binding.hotkey) {
                Ok(()) => self.registered.push(id),
                Err(error) => failures.push(HotkeyRegistrationFailure {
                    binding: binding.clone(),
                    error,
                }),
            }
        }
        failures
    }

    /// 把平台事件转换成命令；与任何绑定都无关的事件返回 `None`。
    pub fn command_for(&self, event: PlatformEvent) -> Option<Command> {
        match event {
            PlatformEvent::HotkeyPressed(id) => self
                .bindings
                .iter()
                .enumerate()
                .find(|(index, _)| hotkey_id(*index) == id)
                .map(|(_, binding)| Command::Capture(binding.capture.clone())),
            PlatformEvent::ExitRequested => Some(Command::Exit),
        }
    }

    /// 处理一个平台事件：转换成命令后执行。
    pub fn handle(&mut self, event: PlatformEvent) -> Result<Flow, CoreError> {
        match self.command_for(event) {
            Some(command) => self.execute(command),
            None => Ok(Flow::Continue),
        }
    }

    /// 执行一条命令。
    pub fn execute(&mut self, command: Command) -> Result<Flow, CoreError> {
        match command {
            Command::Capture(request) => {
                let screenshot =
                    capture::capture(self.platform.frame_source.as_ref(), request.mode)?;
                action::run(&mut self.platform, &screenshot, &request.actions)?;
                Ok(Flow::Continue)
            }
            Command::Exit => {
                self.unregister_hotkeys();
                Ok(Flow::Exit)
            }
        }
    }

    /// 注销所有已注册的热键。尽力而为：某个热键注销失败不影响其他热键，
    /// 进程退出时系统也会回收它们。
    fn unregister_hotkeys(&mut self) {
        for id in self.registered.drain(..) {
            let _ = self.platform.hotkeys.unregister(id);
        }
    }
}

/// 第 `index` 条绑定使用的热键标识。
fn hotkey_id(index: usize) -> HotkeyId {
    HotkeyId(index as u32 + 1)
}
