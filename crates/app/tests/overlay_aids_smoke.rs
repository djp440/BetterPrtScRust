//! 选区遮罩辅助功能真机冒烟：在真实的选区遮罩上显示放大镜、按 C 复制颜色、拖手柄和用方向键
//! 调整选区，检查屏幕上放大镜的颜色、复制的颜色值和最终截图。默认忽略。
//!
//! 运行：`pwsh scripts/smoke-overlay-aids.ps1`（先备份剪贴板，再以
//! `BPS_SMOKE_REAL_CLIPBOARD=1` 运行本测试，结束后恢复剪贴板）。直接
//! `cargo test --release -p bps-app --test overlay_aids_smoke -- --ignored --nocapture`
//! 时不碰系统剪贴板，颜色值只记在替身里。
//!
//! 在本进程里用真实的 Windows 平台创建常驻实例，把遮罩上的输入直接交给它；遮罩包一层记录
//! 最近一次场景。最终截图用自动存储写到临时目录，结束时删除。屏幕会闪现遮罩约 3 秒，期间请勿
//! 操作电脑。`BPS_SMOKE_SHOTS` 指向一个目录时，把放大镜和手柄附近的屏幕截图存成 PNG 供目视检查。
#![cfg(windows)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use bps_core::{
    CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance, SaveDir,
};
use bps_platform::{
    Backdrop, Clipboard, ClipboardItem, Color, Display, DisplayId, FrameSource, Image, Modifiers,
    Notification, Notifier, OverlayInput, OverlayItem, OverlayKey, OverlayScene, PlatformError,
    PlatformEvent, Point, PointerButton, Rect, SelectionOverlay, SystemLanguage, TextMask,
};
use bps_platform_windows::{DxgiFrameSource, EventLoop, dpi};
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 转发给真实遮罩，同时记下最近一次画的场景。
struct RecordingOverlay {
    inner: Box<dyn SelectionOverlay>,
    scene: Rc<RefCell<Option<OverlayScene>>>,
}

impl SelectionOverlay for RecordingOverlay {
    fn open(&mut self, backdrop: Backdrop) -> Result<Vec<Display>, PlatformError> {
        self.inner.open(backdrop)
    }
    fn render(&mut self, scene: &OverlayScene) -> Result<(), PlatformError> {
        *self.scene.borrow_mut() = Some(scene.clone());
        self.inner.render(scene)
    }
    fn frozen_image(&self, display: DisplayId) -> Result<Image, PlatformError> {
        self.inner.frozen_image(display)
    }
    fn frozen_pixels(&self, display: DisplayId, area: Rect) -> Result<Image, PlatformError> {
        self.inner.frozen_pixels(display, area)
    }
    fn show_canvas(&mut self, area: Rect, image: &Image) -> Result<(), PlatformError> {
        self.inner.show_canvas(area, image)
    }
    fn update_canvas(&mut self, at: Point, patch: &Image) -> Result<(), PlatformError> {
        self.inner.update_canvas(at, patch)
    }
    fn rasterize_text(&self, text: &str, font_size: u32) -> Result<TextMask, PlatformError> {
        self.inner.rasterize_text(text, font_size)
    }
    fn set_pass_through(&mut self, area: Option<Rect>) -> Result<(), PlatformError> {
        self.inner.set_pass_through(area)
    }
    fn close(&mut self) {
        *self.scene.borrow_mut() = None;
        self.inner.close();
    }
}

/// 不碰系统剪贴板的替身：只记下最后一次写入的文本。
struct RecordingClipboard(Rc<RefCell<Option<String>>>);

impl Clipboard for RecordingClipboard {
    fn write(&mut self, items: Vec<ClipboardItem>) -> Result<(), PlatformError> {
        let text = items.into_iter().find_map(|item| match item {
            ClipboardItem::Text(text) => Some(text),
            _ => None,
        });
        assert!(text.is_some(), "冒烟测试里只应复制颜色值");
        *self.0.borrow_mut() = text;
        Ok(())
    }
}

struct QuietNotifier;

impl Notifier for QuietNotifier {
    fn notify(&self, notification: Notification) {
        eprintln!("[通知] {}：{}", notification.title, notification.body);
    }
}

struct Chinese;

impl SystemLanguage for Chinese {
    fn preferred_ui_languages(&self) -> Vec<String> {
        vec!["zh-CN".to_owned()]
    }
}

fn input(instance: &mut ResidentInstance, input: OverlayInput) {
    instance
        .handle(PlatformEvent::OverlayInput(input))
        .expect("处理遮罩输入失败");
}

fn move_to(instance: &mut ResidentInstance, at: Point) {
    input(
        instance,
        OverlayInput::PointerMoved {
            position: at,
            modifiers: Modifiers::NONE,
        },
    );
}

fn drag(instance: &mut ResidentInstance, from: Point, to: Point) {
    input(
        instance,
        OverlayInput::PointerPressed {
            button: PointerButton::Left,
            position: from,
            modifiers: Modifiers::NONE,
            double_click: false,
        },
    );
    for step in 1..=8 {
        move_to(
            instance,
            Point::new(
                from.x + (to.x - from.x) * step / 8,
                from.y + (to.y - from.y) * step / 8,
            ),
        );
    }
    input(
        instance,
        OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: to,
            modifiers: Modifiers::NONE,
        },
    );
}

fn key(instance: &mut ResidentInstance, key: OverlayKey, modifiers: Modifiers) {
    input(instance, OverlayInput::KeyPressed { key, modifiers });
}

/// 系统剪贴板里的 Unicode 文本。
fn system_clipboard_text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
            .ok()
            .and_then(|handle| {
                let memory = HGLOBAL(handle.0);
                let pointer = GlobalLock(memory) as *const u16;
                if pointer.is_null() {
                    return None;
                }
                let length = (0..).take_while(|&i| *pointer.add(i) != 0).count();
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length));
                let _ = GlobalUnlock(memory);
                Some(text)
            });
        let _ = CloseClipboard();
        text
    }
}

fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#').filter(|d| d.len() == 6)?;
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some(Color::rgba(channel(0)?, channel(2)?, channel(4)?, 255))
}

fn close_enough(pixel: [u8; 4], color: Color) -> bool {
    pixel[0].abs_diff(color.r) <= 3
        && pixel[1].abs_diff(color.g) <= 3
        && pixel[2].abs_diff(color.b) <= 3
}

fn save_png(path: &Path, image: &Image) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(image.rgba()).unwrap();
}

/// `image`（左上角在虚拟桌面 `origin`）中 `area` 与它重叠的那一块。
fn crop(image: &Image, origin: Point, area: Rect) -> Image {
    let bounds = Rect::new(origin.x, origin.y, image.width(), image.height());
    let area = bounds.intersection(area).unwrap();
    image
        .crop(Rect::new(
            area.x - origin.x,
            area.y - origin.y,
            area.width,
            area.height,
        ))
        .unwrap()
}

#[test]
#[ignore = "真机冒烟：在屏幕上显示选区遮罩、放大镜和手柄"]
fn magnifier_color_and_adjusted_selection_on_real_overlay() {
    assert!(dpi::ensure_per_monitor_v2());
    let real_clipboard = std::env::var_os("BPS_SMOKE_REAL_CLIPBOARD").is_some_and(|v| v == "1");
    let shots = std::env::var_os("BPS_SMOKE_SHOTS").map(PathBuf::from);
    let temp = std::env::temp_dir().join(format!("bps-overlay-aids-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let event_loop = EventLoop::new().unwrap();
    let ui_exe = std::env::current_exe().unwrap();
    let mut platform =
        bps_platform_windows::platform(&event_loop, temp.join("config.toml"), ui_exe);
    let scene = Rc::new(RefCell::new(None));
    let real_overlay = std::mem::replace(&mut platform.overlay, Box::new(NoOverlay));
    platform.overlay = Box::new(RecordingOverlay {
        inner: real_overlay,
        scene: scene.clone(),
    });
    let recorded = Rc::new(RefCell::new(None));
    if !real_clipboard {
        platform.clipboard = Box::new(RecordingClipboard(recorded.clone()));
    }
    platform.notifier = Box::new(QuietNotifier);
    platform.system_language = Box::new(Chinese);
    let mut instance = ResidentInstance::new(platform, Vec::new());

    let frames = DxgiFrameSource::new();
    let primary = frames
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let b = primary.bounds;
    let origin = Point::new(b.x, b.y);
    let center = Point::new(b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
    let mut request = CaptureRequest::new(CaptureMode::Region, vec![PostCaptureAction::AutoSave]);
    request.output.save_dir = SaveDir::Custom(temp.clone());
    instance.execute(Command::Capture(request)).unwrap();

    // 放大镜：移动指针，量每次移动（读冻结帧 + 绘制）的耗时。
    move_to(&mut instance, Point::new(center.x - 20, center.y));
    let moving = Instant::now();
    for dx in 1..=20 {
        move_to(&mut instance, Point::new(center.x - 20 + dx, center.y));
    }
    eprintln!("带放大镜移动指针：每次 {:?}", moving.elapsed() / 20);
    let current = scene.borrow().clone().expect("遮罩应显示着");
    let hex = current
        .items
        .iter()
        .find_map(|item| match item {
            OverlayItem::Text { text, .. } if text.starts_with('#') => Some(text.clone()),
            _ => None,
        })
        .expect("放大镜应显示颜色值");
    let color = parse_hex(&hex).unwrap();
    let cell = current
        .items
        .iter()
        .find_map(|item| match item {
            OverlayItem::Fill { rect, color: c } if *c == color && rect.width == rect.height => {
                Some(*rect)
            }
            _ => None,
        })
        .expect("放大镜里应有对准像素的格子");
    thread::sleep(Duration::from_millis(300));
    let screen = frames.capture_display(primary.id).unwrap();
    let cell_center = Point::new(
        cell.x + cell.width as i32 / 2 - origin.x,
        cell.y + cell.height as i32 / 2 - origin.y,
    );
    let shown = screen.pixel(cell_center.x as u32, cell_center.y as u32);
    eprintln!("放大镜颜色值 {hex}，屏幕上格子中心 {shown:?}，格子 {cell:?}");
    assert!(close_enough(shown, color), "屏幕上放大的像素应是这个颜色");
    if let Some(dir) = &shots {
        let around = Rect::new(center.x - 40, center.y - 40, 320, 320);
        save_png(&dir.join("magnifier.png"), &crop(&screen, origin, around));
    }

    // 按 C 复制颜色值。
    key(&mut instance, OverlayKey::C, Modifiers::NONE);
    let copied = if real_clipboard {
        system_clipboard_text()
    } else {
        recorded.borrow().clone()
    };
    eprintln!(
        "复制的颜色值（{}）：{copied:?}",
        if real_clipboard {
            "系统剪贴板"
        } else {
            "替身"
        }
    );
    assert_eq!(copied.as_deref(), Some(hex.as_str()));

    // 框选、拖右下角手柄、方向键微调。
    let region = Rect::new(center.x - 200, center.y - 150, 400, 300);
    let corner = Point::new(region.x + 399, region.y + 299);
    drag(&mut instance, Point::new(region.x, region.y), corner);
    let handle = Point::new(corner.x + 1, corner.y + 1);
    drag(
        &mut instance,
        handle,
        Point::new(handle.x + 37, handle.y + 23),
    );
    key(&mut instance, OverlayKey::Right, Modifiers::NONE);
    key(&mut instance, OverlayKey::Right, Modifiers::NONE);
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    };
    key(&mut instance, OverlayKey::Up, ctrl);
    let adjusted = Rect::new(region.x + 2, region.y - 1, 437, 324);
    let label = scene
        .borrow()
        .as_ref()
        .and_then(|s| s.labels().next().map(|l| l.text.clone()));
    assert_eq!(label.as_deref(), Some("437 × 324"));
    thread::sleep(Duration::from_millis(300));
    let screen = frames.capture_display(primary.id).unwrap();
    if let Some(dir) = &shots {
        let corner_area = Rect::new(
            adjusted.x + adjusted.width as i32 - 60,
            adjusted.y + adjusted.height as i32 - 60,
            120,
            120,
        );
        save_png(
            &dir.join("handles.png"),
            &crop(&screen, origin, corner_area),
        );
    }

    key(&mut instance, OverlayKey::Enter, Modifiers::NONE);
    assert!(scene.borrow().is_none(), "确定后遮罩应关闭");
    let saved: Vec<PathBuf> = std::fs::read_dir(&temp)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    let image = bps_platform_fake::decode::png(&std::fs::read(&saved[0]).unwrap());
    assert_eq!(
        (image.width(), image.height()),
        (adjusted.width, adjusted.height),
        "截图尺寸应等于调整后的选区"
    );
    // 选区内（避开边框和手柄）屏幕上显示的冻结帧应与存下的截图一致：位置对齐。
    let inner = Rect::new(
        adjusted.x + 10,
        adjusted.y + 10,
        adjusted.width - 20,
        adjusted.height - 20,
    );
    let on_screen = crop(&screen, origin, inner);
    let in_file = image
        .crop(Rect::new(10, 10, inner.width, inner.height))
        .unwrap();
    let differing = on_screen
        .rgba()
        .chunks_exact(4)
        .zip(in_file.rgba().chunks_exact(4))
        .filter(|(a, b)| (0..3).any(|i| a[i].abs_diff(b[i]) > 3))
        .count();
    eprintln!(
        "屏幕上的选区与存下的截图：差值 > 3 的像素 {differing} / {}",
        inner.width * inner.height
    );
    assert!(
        differing * 1000 < (inner.width * inner.height) as usize,
        "存下的截图应与屏幕上调整后的选区一致"
    );
    // 放大镜对准的像素在截图里也是复制的颜色。
    let at = Point::new(center.x - adjusted.x, center.y - adjusted.y);
    let pixel = image.pixel(at.x as u32, at.y as u32);
    assert_eq!([pixel[0], pixel[1], pixel[2]], [color.r, color.g, color.b]);
    std::fs::remove_dir_all(&temp).unwrap();
}

/// 占位用，替换遮罩时临时放进去。
struct NoOverlay;

impl SelectionOverlay for NoOverlay {
    fn open(&mut self, _: Backdrop) -> Result<Vec<Display>, PlatformError> {
        unreachable!()
    }
    fn render(&mut self, _: &OverlayScene) -> Result<(), PlatformError> {
        unreachable!()
    }
    fn frozen_image(&self, _: DisplayId) -> Result<Image, PlatformError> {
        unreachable!()
    }
    fn show_canvas(&mut self, _: Rect, _: &Image) -> Result<(), PlatformError> {
        unreachable!()
    }
    fn update_canvas(&mut self, _: Point, _: &Image) -> Result<(), PlatformError> {
        unreachable!()
    }
    fn rasterize_text(&self, _: &str, _: u32) -> Result<TextMask, PlatformError> {
        unreachable!()
    }
    fn set_pass_through(&mut self, _: Option<Rect>) -> Result<(), PlatformError> {
        unreachable!()
    }
    fn close(&mut self) {}
}
