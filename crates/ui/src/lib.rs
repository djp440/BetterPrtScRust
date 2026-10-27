//! BetterPrtSc 的独立窗口界面（egui）。
//!
//! 只在按需启动的界面子进程（`BetterPrtSc.exe --settings` 等）里运行，用户关掉窗口进程就退出；
//! 常驻实例自己不初始化任何界面库（ADR 0005）。每种窗口一个入口函数，例如
//! [`run_settings_window`]、[`run_result_panel`]，运行到用户关闭窗口为止。
//!
//! 平台相关的能力（字体、图标、打开文件、查询常驻实例……）由调用方注入：所有窗口共用的放在
//! [`UiEnvironment`]，某个窗口特有的放在它自己的环境里（如 [`SettingsEnvironment`]）。
//! 界面子进程需要常驻实例的数据时，经命名管道发请求查询（`bps_core::Request`）。

mod clipboard;
mod hotkey_recording;
mod result_panel;
mod settings_window;
mod store;

use std::ffi::OsStr;
use std::sync::Arc;

use bps_core::{Language, Texts};
use eframe::egui;

pub use clipboard::{ReadClipboardText, SystemClipboard, WriteClipboardText};
pub use hotkey_recording::{KeyStream, PauseHotkeys, StartKeyCapture};
pub use result_panel::{ResultPanelEnvironment, run_result_panel};
pub use settings_window::{
    PickFolder, ProbeRecording, RecordingCapabilities, SettingsEnvironment, run_settings_window,
};

/// 界面用的一个字体文件（或字体集里的一个字体）。
#[derive(Clone, Copy)]
pub struct UiFont {
    pub name: &'static str,
    /// 字体文件的内容，通常是只读映射的系统字体。
    pub data: &'static [u8],
    /// 字体集（`.ttc`）里的第几个字体。
    pub index: u32,
}

/// 窗口图标（RGBA，逐行排列）。
#[derive(Clone)]
pub struct WindowIcon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 用默认程序打开文件、文件夹或网址；失败时返回给人看的说明。
pub type OpenWithDefaultProgram = Arc<dyn Fn(&OsStr) -> Result<(), String> + Send + Sync>;

/// 所有界面窗口共用的环境。
#[derive(Clone)]
pub struct UiEnvironment {
    /// 按优先顺序排列：前面的字体缺字时依次用后面的。至少要有一个。
    pub fonts: Vec<UiFont>,
    pub icon: Option<WindowIcon>,
    /// 系统偏好的界面语言（如 `zh-CN`），界面语言设为“跟随系统”时据此选择。
    pub system_languages: Vec<String>,
    pub open: OpenWithDefaultProgram,
    /// 系统剪贴板：文本框里的复制、剪切、粘贴经它与其他程序互通。
    pub clipboard: SystemClipboard,
}

impl UiEnvironment {
    /// 按 `language` 设置选出的界面文本。
    pub fn texts(&self, language: Language) -> &'static Texts {
        Texts::of(language.resolve(&self.system_languages))
    }
}

/// 窗口的初始外观。
struct WindowSpec {
    title: String,
    inner_size: [f32; 2],
    min_inner_size: [f32; 2],
}

/// 打开一个窗口并运行到它被关闭。字体、字号、图标按 `environment` 统一设置。
fn run_window<A: eframe::App + 'static>(
    environment: &UiEnvironment,
    spec: WindowSpec,
    create: impl FnOnce(&egui::Context) -> A,
) -> Result<(), String> {
    if environment.fonts.is_empty() {
        return Err("找不到可用的界面字体".to_owned());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(spec.title)
        .with_inner_size(spec.inner_size)
        .with_min_inner_size(spec.min_inner_size);
    if let Some(icon) = &environment.icon {
        viewport = viewport.with_icon(egui::IconData {
            rgba: icon.rgba.clone(),
            width: icon.width,
            height: icon.height,
        });
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: true,
        // 字体由调用方提供，不在运行时另找系统字体。
        system_font_fallback: false,
        ..Default::default()
    };
    let fonts = environment.fonts.clone();
    let clipboard = environment.clipboard.clone();
    eframe::run_native(
        "BetterPrtSc",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx, &fonts);
            cc.egui_ctx.all_styles_mut(|style| {
                for (text_style, font) in style.text_styles.iter_mut() {
                    font.size = match text_style {
                        egui::TextStyle::Heading => 19.0,
                        egui::TextStyle::Small => 12.0,
                        _ => 14.0,
                    };
                }
                style.spacing.item_spacing = egui::vec2(8.0, 6.0);
                style.spacing.button_padding = egui::vec2(8.0, 3.0);
            });
            Ok(Box::new(clipboard.bridge(create(&cc.egui_ctx))))
        }),
    )
    .map_err(|e| e.to_string())
}

fn install_fonts(ctx: &egui::Context, fonts: &[UiFont]) {
    let mut definitions = egui::FontDefinitions::empty();
    for font in fonts {
        let mut data = egui::FontData::from_static(font.data);
        data.index = font.index;
        definitions
            .font_data
            .insert(font.name.to_owned(), Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            definitions
                .families
                .entry(family)
                .or_default()
                .push(font.name.to_owned());
        }
    }
    ctx.set_fonts(definitions);
}
