//! 系统“截图工具”对 PrtSc 键的占用：读取“使用 Print Screen 键打开屏幕截图”这项设置
//! （`HKCU\Control Panel\Keyboard` 的 `PrintScreenKeyForSnippingEnabled`），只读不改。

use std::rc::Rc;

use bps_platform::{PlatformError, SystemScreenshotTool};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::event_loop::Shared;
use crate::registry::{self, RegistryWatch};

const KEYBOARD_KEY: &str = r"Control Panel\Keyboard";
const SNIPPING_VALUE: &str = "PrintScreenKeyForSnippingEnabled";
/// Windows 11 22H2（22621）起，这项设置在用户没改过（注册表里没有这个值）时默认开启。
const DEFAULT_ON_SINCE_BUILD: u32 = 22621;

/// [`SystemScreenshotTool`] 的 Windows 实现。监视由创建它的事件循环负责：设置所在的注册表键
/// 有变化时，事件循环稍等片刻（让系统先释放 PrtSc）再递交 `PrintScreenKeySettingChanged`。
pub struct WindowsScreenshotTool {
    shared: Rc<Shared>,
}

impl WindowsScreenshotTool {
    pub(crate) fn new(shared: Rc<Shared>) -> Self {
        Self { shared }
    }
}

impl SystemScreenshotTool for WindowsScreenshotTool {
    fn takes_print_screen_key(&self) -> Result<bool, PlatformError> {
        match registry::read_dword(HKEY_CURRENT_USER, KEYBOARD_KEY, SNIPPING_VALUE)? {
            Some(value) => Ok(value != 0),
            None => Ok(windows_build()? >= DEFAULT_ON_SINCE_BUILD),
        }
    }

    fn watch(&mut self) -> Result<(), PlatformError> {
        let watch = RegistryWatch::new(HKEY_CURRENT_USER, KEYBOARD_KEY)?;
        *self.shared.keyboard_watch.borrow_mut() = Some(watch);
        Ok(())
    }
}

/// 系统的内部版本号（如 26100）。
fn windows_build() -> Result<u32, PlatformError> {
    let text = registry::read_string(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentBuildNumber",
    )?
    .unwrap_or_default();
    text.trim()
        .parse()
        .map_err(|_| PlatformError::new(format!("无法识别系统版本号“{text}”")))
}
