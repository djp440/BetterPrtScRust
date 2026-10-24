//! 结果面板子进程（`BetterPrtSc.exe --result-panel`）：从标准输入读出常驻实例交来的截图，
//! 在本机识别其中的文字（`Windows.Media.Ocr`）并显示结果，用户关掉窗口即退出。
//!
//! OCR 只在这个子进程里加载，常驻实例不加载它。界面语言按配置文件里的设置。

use std::error::Error;
use std::sync::Arc;

use bps_core::{Config, Language};
use bps_platform_windows::{WindowsTextRecognizer, read_screenshot};
use bps_ui::{ResultPanelEnvironment, run_result_panel};

use crate::{data_location, settings};

pub fn run() -> Result<(), Box<dyn Error>> {
    let screenshot = read_screenshot(&mut std::io::stdin().lock())?;
    run_result_panel(ResultPanelEnvironment {
        ui: settings::ui_environment(),
        language: configured_language(),
        screenshot,
        recognizer: Arc::new(WindowsTextRecognizer),
    })?;
    Ok(())
}

/// 配置文件里的界面语言；读不到或有错误时跟随系统。
fn configured_language() -> Language {
    data_location()
        .ok()
        .and_then(|location| std::fs::read_to_string(location.config_file()).ok())
        .and_then(|text| Config::parse(&text).ok())
        .map(|config| config.language)
        .unwrap_or_default()
}
