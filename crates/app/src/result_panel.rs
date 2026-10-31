//! 结果面板子进程（`BetterPrtSc.exe --result-panel`）：从标准输入读出常驻实例交来的截图，
//! 在本机识别其中的文字（`Windows.Media.Ocr`）并显示结果，用户关掉窗口即退出。
//!
//! 带 `--auto-copy` 时不显示界面：识别后把文字复制到剪贴板，再经命名管道把结果交回常驻实例，
//! 由它用托盘通知告诉用户（OCR 开了“识别后自动复制”时）。
//!
//! OCR 只在这个子进程里加载，常驻实例不加载它。界面语言和翻译服务按配置文件里的设置；
//! 只有用户点“翻译”时才联网（ADR 0004）。

use std::error::Error;
use std::sync::Arc;

use bps_core::{Config, Request, WhenNotRunning, copy_recognized_text, forward, set_ocr_auto_copy};
use bps_platform::Image;
use bps_platform_windows::{
    AUTO_COPY_ARGUMENT, PipeChannel, TRANSLATE_ARGUMENT, WinHttpTransport, WindowsClipboard,
    WindowsTextRecognizer, read_screenshot, resident_pipe_name,
};
use bps_ui::{ResultPanelEnvironment, run_result_panel};

use crate::{data_location, settings};

pub fn run() -> Result<(), Box<dyn Error>> {
    // 常驻实例用 `--translate` 要求识别完成后直接翻译（截图界面上的“翻译”按钮）。
    let translate = std::env::args().any(|a| a == TRANSLATE_ARGUMENT);
    let auto_copy = std::env::args().any(|a| a == AUTO_COPY_ARGUMENT);
    let screenshot = read_screenshot(&mut std::io::stdin().lock())?;
    if auto_copy {
        return copy_without_panel(screenshot, translate);
    }
    let config = configured();
    run_result_panel(ResultPanelEnvironment {
        ui: settings::ui_environment(),
        language: config.as_ref().map(|c| c.language).unwrap_or_default(),
        screenshot,
        recognizer: Arc::new(WindowsTextRecognizer),
        translation: config.as_ref().and_then(|c| c.translation.clone()),
        http: Arc::new(WinHttpTransport),
        translate,
        auto_copy: config.is_some_and(|c| c.ocr_auto_copy),
        save_auto_copy: Box::new(save_auto_copy),
    })?;
    Ok(())
}

/// 识别、复制，把结果交回常驻实例去通知用户。
fn copy_without_panel(screenshot: Image, translate: bool) -> Result<(), Box<dyn Error>> {
    let mut clipboard = WindowsClipboard::new(None);
    let copy = if translate {
        let config = configured().unwrap_or_default();
        let locale = config
            .language
            .resolve(&settings::ui_environment().system_languages);
        bps_core::copy_translated_text(
            screenshot,
            &WindowsTextRecognizer,
            &mut clipboard,
            config.translation.as_ref(),
            locale,
            &WinHttpTransport,
        )
    } else {
        copy_recognized_text(screenshot, &WindowsTextRecognizer, &mut clipboard)
    };
    eprintln!("[OCR 自动复制] {copy:?}");
    let exe = std::env::current_exe()?;
    let mut channel = PipeChannel::new(&resident_pipe_name(), exe);
    forward(
        &Request::OcrCopied(copy),
        &mut channel,
        WhenNotRunning::GiveUp,
    )?;
    Ok(())
}

/// 结果面板上切换了“识别后自动复制”：立即写回配置文件（常驻实例会自动重新加载）。
fn save_auto_copy(on: bool) -> Result<(), String> {
    let path = data_location().map_err(|e| e.to_string())?.config_file();
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let text = set_ocr_auto_copy(&text, on).ok_or("config.toml 有错误，无法写回")?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// 配置文件的内容（界面语言、翻译服务）；读不到或有错误时为 `None`（语言跟随系统、不能翻译）。
fn configured() -> Option<Config> {
    data_location()
        .ok()
        .and_then(|location| std::fs::read_to_string(location.config_file()).ok())
        .and_then(|text| Config::parse(&text).ok())
}
