//! 独立窗口界面：设置界面等放在按需启动、用完即退出的子进程里（`BetterPrtSc.exe --settings`），
//! 常驻实例自己不加载任何界面库（ADR 0005）。

use std::path::PathBuf;
use std::process::{Child, Command};

use bps_platform::{PlatformError, StandaloneWindow, WindowLauncher};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::BOOL;

/// 子进程的命令行参数：`BetterPrtSc.exe <参数>` 打开对应的窗口。
pub fn window_argument(window: StandaloneWindow) -> &'static str {
    match window {
        StandaloneWindow::Settings => "--settings",
    }
}

/// 以子进程打开独立窗口；同一种窗口同时只开一个。
pub struct ChildProcessWindows {
    exe: PathBuf,
    children: Vec<(StandaloneWindow, Child)>,
}

impl ChildProcessWindows {
    /// `exe` 是带界面的程序，通常就是常驻实例自己的 `BetterPrtSc.exe`。
    pub fn new(exe: PathBuf) -> Self {
        Self {
            exe,
            children: Vec::new(),
        }
    }
}

impl WindowLauncher for ChildProcessWindows {
    fn open(&mut self, window: StandaloneWindow) -> Result<(), PlatformError> {
        // 回收已经退出的子进程。
        self.children
            .retain_mut(|(_, child)| matches!(child.try_wait(), Ok(None)));
        if let Some((_, child)) = self.children.iter().find(|(w, _)| *w == window) {
            bring_to_front(child.id());
            return Ok(());
        }
        // 标准输入输出沿用常驻实例的：debug 版的日志进同一个控制台，release 版没有控制台。
        let child = Command::new(&self.exe)
            .arg(window_argument(window))
            .spawn()
            .map_err(|e| PlatformError::new(format!("无法启动 {}：{e}", self.exe.display())))?;
        // 常驻实例刚响应了用户操作（托盘菜单或再次启动），把前台权限转给子进程，
        // 新窗口才能出现在最前面。
        let _ = unsafe { AllowSetForegroundWindow(child.id()) };
        self.children.push((window, child));
        Ok(())
    }
}

/// 把进程 `pid` 的可见顶层窗口带到前台（最小化时先还原）。
fn bring_to_front(pid: u32) {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            search.found = Some(hwnd);
            return false.into();
        }
        true.into()
    }
    let mut search = Search { pid, found: None };
    // 回调返回 FALSE 提前结束时 EnumWindows 也返回“失败”，忽略即可。
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    if let Some(hwnd) = search.found {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}
