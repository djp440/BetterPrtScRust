use std::sync::{Arc, Mutex};

use bps_platform::{
    Autostart, AutostartEntry, AutostartMethod, Elevation, PlatformError, SystemScreenshotTool,
};

/// 开机自启登记、进程权限与系统截图工具设置的共享状态。
#[derive(Default)]
pub(crate) struct SystemState {
    /// 已登记的开机自启，每种方式最多一条。
    pub entries: Vec<AutostartEntry>,
    /// 设为 `Some` 时，登记和移除开机自启都以这条消息失败。
    pub autostart_error: Option<String>,
    /// 本进程是否以管理员权限运行。
    pub elevated: bool,
    /// 用户在 UAC 提示里点了“否”。
    pub elevation_declined: bool,
    /// 以管理员身份重新启动的次数（不论用户是否同意）。
    pub administrator_restarts: usize,
    /// 系统设置把 PrtSc 键交给了系统截图工具。
    pub screenshot_tool_takes_print_screen: bool,
    pub screenshot_tool_watched: bool,
}

impl SystemState {
    pub fn sorted_entries(&self) -> Vec<AutostartEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by_key(|e| e.method != AutostartMethod::Normal);
        entries
    }

    fn check(&self, method: AutostartMethod) -> Result<(), PlatformError> {
        if let Some(message) = &self.autostart_error {
            return Err(PlatformError::new(message.clone()));
        }
        if method == AutostartMethod::Elevated && !self.elevated {
            return Err(PlatformError::new("需要管理员权限"));
        }
        Ok(())
    }
}

pub(crate) struct FakeAutostart(pub Arc<Mutex<SystemState>>);

impl Autostart for FakeAutostart {
    fn entries(&self) -> Result<Vec<AutostartEntry>, PlatformError> {
        Ok(self.0.lock().unwrap().sorted_entries())
    }

    fn enable(&mut self, method: AutostartMethod) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        state.check(method)?;
        state.entries.retain(|e| e.method != method);
        state.entries.push(AutostartEntry {
            method,
            this_program: true,
        });
        Ok(())
    }

    fn disable(&mut self, method: AutostartMethod) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        if !state.entries.iter().any(|e| e.method == method) {
            return Ok(());
        }
        state.check(method)?;
        state.entries.retain(|e| e.method != method);
        Ok(())
    }
}

pub(crate) struct FakeElevation(pub Arc<Mutex<SystemState>>);

impl Elevation for FakeElevation {
    fn is_elevated(&self) -> bool {
        self.0.lock().unwrap().elevated
    }

    fn restart_as_administrator(&mut self) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        state.administrator_restarts += 1;
        if state.elevation_declined {
            Err(PlatformError::new("用户取消了操作"))
        } else {
            Ok(())
        }
    }
}

pub(crate) struct FakeScreenshotTool(pub Arc<Mutex<SystemState>>);

impl SystemScreenshotTool for FakeScreenshotTool {
    fn takes_print_screen_key(&self) -> Result<bool, PlatformError> {
        Ok(self.0.lock().unwrap().screenshot_tool_takes_print_screen)
    }

    fn watch(&mut self) -> Result<(), PlatformError> {
        self.0.lock().unwrap().screenshot_tool_watched = true;
        Ok(())
    }
}
