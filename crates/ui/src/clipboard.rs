//! 把 egui 文本框的复制、剪切、粘贴接到系统剪贴板上。
//!
//! eframe 没有开 `clipboard` 特性（那要多带 arboard 和图像解码库），这时 egui-winit 只有一个
//! 进程内的剪贴板：Ctrl+C 复制的文字到不了其他程序，Ctrl+V 也粘贴不进其他程序复制的文字。
//! [`SystemClipboard::bridge`] 包住窗口的 `eframe::App`，每一帧：
//! - 把界面发出的“复制文字”命令写进系统剪贴板；
//! - 把 Ctrl+V 产生的粘贴事件换成系统剪贴板里的文字。
//!
//! egui-winit 在进程内剪贴板为空时不产生粘贴事件，所以它的进程内剪贴板总是放一个占位符。

use std::sync::Arc;

use eframe::egui;

/// 读出系统剪贴板里的文字；没有文字或读取失败时为 `None`。
pub type ReadClipboardText = Arc<dyn Fn() -> Option<String> + Send + Sync>;
/// 把文字放进系统剪贴板；失败时返回给人看的说明。
pub type WriteClipboardText = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// 读写系统剪贴板里的文字。
#[derive(Clone)]
pub struct SystemClipboard {
    pub read_text: ReadClipboardText,
    pub write_text: WriteClipboardText,
}

/// 留在 egui-winit 进程内剪贴板里的占位符，只为让它在 Ctrl+V 时产生粘贴事件。
const PLACEHOLDER: &str = "\u{FFFC}";

impl SystemClipboard {
    /// 包住 `app`，让它的文本框与系统剪贴板互通。
    pub(crate) fn bridge<A: eframe::App>(&self, app: A) -> ClipboardBridge<A> {
        ClipboardBridge {
            app,
            clipboard: self.clone(),
            primed: false,
        }
    }
}

pub(crate) struct ClipboardBridge<A> {
    app: A,
    clipboard: SystemClipboard,
    /// 已经在 egui-winit 的进程内剪贴板里放了占位符。
    primed: bool,
}

impl<A: eframe::App> eframe::App for ClipboardBridge<A> {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let primed = std::mem::replace(&mut self.primed, true);
        ui.ctx().output_mut(|output| {
            for command in &mut output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    if let Err(e) = (self.clipboard.write_text)(text) {
                        eprintln!("复制到剪贴板失败：{e}");
                    }
                    *text = PLACEHOLDER.to_owned();
                }
            }
            if !primed {
                output
                    .commands
                    .push(egui::OutputCommand::CopyText(PLACEHOLDER.to_owned()));
            }
        });
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
        if !raw_input
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::Paste(_)))
        {
            return;
        }
        let text = (self.clipboard.read_text)().filter(|text| !text.is_empty());
        raw_input.events.retain_mut(|event| match event {
            egui::Event::Paste(pasted) => match &text {
                Some(text) => {
                    pasted.clone_from(text);
                    true
                }
                None => false,
            },
            _ => true,
        });
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.app.on_exit(gl);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }
}
