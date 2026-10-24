//! 系统的界面语言。

use bps_platform::SystemLanguage;
use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::core::PWSTR;

/// 当前用户偏好的界面语言。
pub struct WindowsSystemLanguage;

impl SystemLanguage for WindowsSystemLanguage {
    fn preferred_ui_languages(&self) -> Vec<String> {
        user_ui_languages()
    }
}

/// 当前用户偏好的界面语言（如 `zh-CN`、`en-US`），按优先级排列；读取失败时为空。
pub fn user_ui_languages() -> Vec<String> {
    let mut count = 0u32;
    let mut length = 0u32;
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut length).is_err() {
            return Vec::new();
        }
        let mut buffer = vec![0u16; length as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
        .is_err()
        {
            return Vec::new();
        }
        // 以 NUL 分隔、两个 NUL 结尾的字符串列表。
        buffer
            .split(|&c| c == 0)
            .filter(|name| !name.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}
