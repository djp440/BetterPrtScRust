//! 独立窗口界面：设置界面、结果面板放在按需启动、用完即退出的子进程里
//! （`BetterPrtSc.exe --settings`、`BetterPrtSc.exe --result-panel`），
//! 常驻实例自己不加载任何界面库（ADR 0005），也不加载 OCR。

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;

use bps_platform::{Image, PlatformError, StandaloneWindow, WindowLauncher};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SW_RESTORE, SetForegroundWindow, ShowWindow,
};
use windows::core::BOOL;

/// 设置界面子进程的命令行参数。
pub const SETTINGS_ARGUMENT: &str = "--settings";
/// 结果面板子进程的命令行参数。要识别的截图经子进程的标准输入传入，
/// 子进程用 [`read_screenshot`] 读取。
pub const RESULT_PANEL_ARGUMENT: &str = "--result-panel";

/// 子进程的命令行参数：`BetterPrtSc.exe <参数>` 打开对应的窗口。
pub fn window_argument(window: &StandaloneWindow) -> &'static str {
    match window {
        StandaloneWindow::Settings => SETTINGS_ARGUMENT,
        StandaloneWindow::ResultPanel { .. } => RESULT_PANEL_ARGUMENT,
    }
}

/// 以子进程打开独立窗口。设置界面同时只开一个；结果面板每次都开一个新的。
pub struct ChildProcessWindows {
    exe: PathBuf,
    settings: Option<Child>,
}

impl ChildProcessWindows {
    /// `exe` 是带界面的程序，通常就是常驻实例自己的 `BetterPrtSc.exe`。
    pub fn new(exe: PathBuf) -> Self {
        Self {
            exe,
            settings: None,
        }
    }

    fn spawn(&self, argument: &str, stdin: Stdio) -> Result<Child, PlatformError> {
        // 标准输出沿用常驻实例的：debug 版的日志进同一个控制台，release 版没有控制台。
        let child = Command::new(&self.exe)
            .arg(argument)
            .stdin(stdin)
            .spawn()
            .map_err(|e| PlatformError::new(format!("无法启动 {}：{e}", self.exe.display())))?;
        // 常驻实例刚响应了用户操作（托盘菜单、再次启动、在选区遮罩上确定），把前台权限转给子进程，
        // 新窗口才能出现在最前面。
        let _ = unsafe { AllowSetForegroundWindow(child.id()) };
        Ok(child)
    }
}

impl WindowLauncher for ChildProcessWindows {
    fn open(&mut self, window: StandaloneWindow) -> Result<(), PlatformError> {
        match window {
            StandaloneWindow::Settings => {
                if let Some(child) = &mut self.settings
                    && matches!(child.try_wait(), Ok(None))
                {
                    bring_to_front(child.id());
                    return Ok(());
                }
                self.settings = Some(self.spawn(SETTINGS_ARGUMENT, Stdio::inherit())?);
                Ok(())
            }
            StandaloneWindow::ResultPanel { screenshot } => {
                let mut child = self.spawn(RESULT_PANEL_ARGUMENT, Stdio::piped())?;
                let mut stdin = child.stdin.take().expect("标准输入是管道");
                // 截图可能有几十 MB，另开线程写，不阻塞常驻实例的事件循环；写完关闭管道。
                // 子进程提前退出时写入失败，什么也不用做。
                thread::Builder::new()
                    .name("result-panel-input".to_owned())
                    .spawn(move || {
                        let _ = write_screenshot(&screenshot, &mut stdin);
                    })
                    .map_err(|e| PlatformError::new(format!("无法把截图交给结果面板：{e}")))?;
                // 不等待结果面板退出；丢弃句柄不影响子进程。
                drop(child);
                Ok(())
            }
        }
    }
}

/// 截图在管道里的格式：魔数、宽、高（u32 小端），然后是 RGBA 像素。
const SCREENSHOT_MAGIC: &[u8; 8] = b"BPSIMG1\n";
/// 读入的截图最多这么多像素（16384 × 16384），防止格式错误时分配巨量内存。
const MAX_SCREENSHOT_PIXELS: u64 = 16384 * 16384;

/// 把截图写进管道，供结果面板子进程用 [`read_screenshot`] 读取。
pub fn write_screenshot(screenshot: &Image, output: &mut impl Write) -> io::Result<()> {
    output.write_all(SCREENSHOT_MAGIC)?;
    output.write_all(&screenshot.width().to_le_bytes())?;
    output.write_all(&screenshot.height().to_le_bytes())?;
    output.write_all(screenshot.rgba())?;
    output.flush()
}

/// 读取常驻实例用 [`write_screenshot`] 写进管道（结果面板子进程的标准输入）的截图。
pub fn read_screenshot(input: &mut impl Read) -> Result<Image, PlatformError> {
    let fail = |e: io::Error| PlatformError::new(format!("读取要识别的截图失败：{e}"));
    let mut header = [0u8; 16];
    input.read_exact(&mut header).map_err(fail)?;
    if &header[..8] != SCREENSHOT_MAGIC {
        return Err(PlatformError::new("读取要识别的截图失败：格式不对"));
    }
    let width = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_SCREENSHOT_PIXELS {
        return Err(PlatformError::new(format!(
            "读取要识别的截图失败：尺寸 {width}×{height} 不合理"
        )));
    }
    let mut rgba = vec![0u8; pixels as usize * 4];
    input.read_exact(&mut rgba).map_err(fail)?;
    Ok(Image::from_rgba(width, height, rgba))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_survives_the_pipe_format() {
        let pixels = (0..3 * 2 * 4).map(|i| i as u8).collect();
        let screenshot = Image::from_rgba(3, 2, pixels);
        let mut bytes = Vec::new();
        write_screenshot(&screenshot, &mut bytes).unwrap();

        assert_eq!(read_screenshot(&mut bytes.as_slice()).unwrap(), screenshot);
    }

    #[test]
    fn truncated_screenshot_is_an_error() {
        let screenshot = Image::from_rgba(2, 2, vec![7; 16]);
        let mut bytes = Vec::new();
        write_screenshot(&screenshot, &mut bytes).unwrap();
        bytes.truncate(bytes.len() - 1);

        assert!(read_screenshot(&mut bytes.as_slice()).is_err());
        assert!(read_screenshot(&mut &b"not an image"[..]).is_err());
    }
}
