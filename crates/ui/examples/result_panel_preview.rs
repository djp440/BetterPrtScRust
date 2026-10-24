//! 结果面板的界面预览：用一个假的文字识别显示各种状态，供目视检查布局与文字。
//! 不碰剪贴板（复制只打印到控制台），不发出网络请求。
//!
//! ```text
//! cargo run -p bps-ui --example result_panel_preview -- [recognized|slow|failed|no-language-pack] [zh-CN|en]
//! ```

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bps_core::Language;
use bps_platform::{Image, PlatformError, RecognitionLanguage, RecognizedLine, TextRecognizer};
use bps_ui::{ResultPanelEnvironment, SystemClipboard, UiEnvironment, UiFont, run_result_panel};

struct PreviewRecognizer {
    scenario: String,
}

impl TextRecognizer for PreviewRecognizer {
    fn languages(&self) -> Result<Vec<RecognitionLanguage>, PlatformError> {
        if self.scenario == "no-language-pack" {
            return Ok(Vec::new());
        }
        Ok(vec![
            RecognitionLanguage {
                tag: "en-US".to_owned(),
                name: "English (United States)".to_owned(),
            },
            RecognitionLanguage {
                tag: "zh-Hans-CN".to_owned(),
                name: "中文(中华人民共和国)".to_owned(),
            },
        ])
    }

    fn default_language(&self) -> Option<String> {
        (self.scenario != "no-language-pack").then(|| "zh-Hans-CN".to_owned())
    }

    fn recognize(&self, _: &Image, language: &str) -> Result<Vec<RecognizedLine>, PlatformError> {
        if self.scenario == "slow" {
            thread::sleep(Duration::from_secs(3));
        }
        if self.scenario == "failed" {
            return Err(PlatformError::new("识别引擎返回了错误（预览）"));
        }
        let words: &[&[&str]] = if language == "en-US" {
            &[
                &["The", "quick", "brown", "fox"],
                &["jumps", "over", "the", "lazy", "dog."],
            ]
        } else {
            &[
                &["BetterPrtSc", "截", "图", "工", "具"],
                &[
                    "使", "用", "Windows", "识", "别", "文", "字", "，", "版", "本", "2", "．", "0",
                ],
            ]
        };
        Ok(words
            .iter()
            .map(|line| line.iter().map(|w| (*w).to_owned()).collect())
            .collect())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scenario = args
        .first()
        .cloned()
        .unwrap_or_else(|| "recognized".to_owned());
    let language = match args.get(1).map(String::as_str) {
        Some("en") => Language::En,
        Some("zh-CN") => Language::ZhCn,
        _ => Language::Auto,
    };
    let fonts = [
        ("Segoe UI", "segoeui.ttf", 0),
        ("Microsoft YaHei UI", "msyh.ttc", 1),
    ]
    .into_iter()
    .filter_map(|(name, file, index)| {
        bps_platform_windows::map_system_font(file).map(|data| UiFont { name, data, index })
    })
    .collect();
    let ui = UiEnvironment {
        fonts,
        icon: None,
        system_languages: bps_platform_windows::user_ui_languages(),
        open: Arc::new(|target| {
            println!("打开 {}", target.to_string_lossy());
            Ok(())
        }),
        clipboard: SystemClipboard {
            read_text: Arc::new(|| Some("（预览：粘贴的文字）".to_owned())),
            write_text: Arc::new(|text| {
                println!("复制：{text:?}");
                Ok(())
            }),
        },
    };
    let result = run_result_panel(ResultPanelEnvironment {
        ui,
        language,
        screenshot: Image::from_rgba(1, 1, vec![255; 4]),
        recognizer: Arc::new(PreviewRecognizer { scenario }),
    });
    if let Err(e) = result {
        eprintln!("{e}");
    }
}
