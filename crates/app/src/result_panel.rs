//! 结果面板子进程（`BetterPrtSc.exe --result-panel`）：从标准输入读出常驻实例交来的截图，
//! 在本机识别其中的文字（`Windows.Media.Ocr`）并显示结果，用户关掉窗口即退出。
//!
//! OCR 只在这个子进程里加载，常驻实例不加载它。界面语言和翻译服务按配置文件里的设置；
//! 只有用户点“翻译”时才联网（ADR 0004）。

use std::error::Error;
use std::sync::Arc;

use bps_core::Config;
use bps_platform_windows::{
    TRANSLATE_ARGUMENT, WinHttpTransport, WindowsTextRecognizer, read_screenshot,
};
use bps_ui::{ResultPanelEnvironment, run_result_panel};

use crate::{data_location, settings};

pub fn run() -> Result<(), Box<dyn Error>> {
    // 常驻实例用 `--translate` 要求识别完成后直接翻译（截图界面上的“翻译”按钮）。
    let translate = std::env::args().any(|a| a == TRANSLATE_ARGUMENT);
    let screenshot = read_screenshot(&mut std::io::stdin().lock())?;
    let config = configured();
    run_result_panel(ResultPanelEnvironment {
        ui: settings::ui_environment(),
        language: config.as_ref().map(|c| c.language).unwrap_or_default(),
        screenshot,
        recognizer: Arc::new(WindowsTextRecognizer),
        translation: config.and_then(|c| c.translation),
        http: Arc::new(WinHttpTransport),
        translate,
    })?;
    Ok(())
}

/// 配置文件的内容（界面语言、翻译服务）；读不到或有错误时为 `None`（语言跟随系统、不能翻译）。
fn configured() -> Option<Config> {
    data_location()
        .ok()
        .and_then(|location| std::fs::read_to_string(location.config_file()).ok())
        .and_then(|text| Config::parse(&text).ok())
}
