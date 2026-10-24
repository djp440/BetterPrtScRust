//! 标注真机冒烟：在真实的选区遮罩上框选、画框和箭头，检查屏幕上显示的标注与存下的截图
//! 一致，以及“另存为”对话框显示在遮罩之上。默认忽略。
//!
//! 运行（release 下标注的合成才是真实速度）：
//! `cargo test --release -p bps-app --test annotation_smoke -- --ignored --nocapture`
//!
//! 在本进程里用真实的 Windows 平台创建常驻实例（不启动 BetterPrtSc.exe），把遮罩上的输入
//! 直接交给常驻实例；遮罩包一层记录最近一次场景，用来找工具条按钮的位置。剪贴板、通知换成
//! 不出声的替身，不动用户的剪贴板、不弹通知；自动存储写到临时目录，结束时删除。
//! 屏幕会闪现遮罩约 5 秒，期间请勿操作电脑。`BPS_SMOKE_SHOTS` 指向一个目录时，把带工具条
//! 和带对话框的屏幕截图存成 PNG 供目视检查。
#![cfg(windows)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bps_core::{
    CaptureMode, CaptureRequest, Command, PostCaptureAction, ResidentInstance, SaveDir,
};
use bps_platform::{
    Backdrop, Clipboard, ClipboardItem, Display, DisplayId, FrameSource, Image, Modifiers,
    Notification, Notifier, OverlayInput, OverlayScene, PlatformError, PlatformEvent, Point,
    PointerButton, Rect, SelectionOverlay,
};
use bps_platform_windows::{DxgiFrameSource, EventLoop, dpi};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, WM_CLOSE, WS_EX_TOPMOST,
};
use windows::core::{PCWSTR, w};

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
    fn show_canvas(&mut self, area: Rect, image: &Image) -> Result<(), PlatformError> {
        self.inner.show_canvas(area, image)
    }
    fn update_canvas(&mut self, at: Point, patch: &Image) -> Result<(), PlatformError> {
        self.inner.update_canvas(at, patch)
    }
    fn close(&mut self) {
        *self.scene.borrow_mut() = None;
        self.inner.close();
    }
}

struct QuietNotifier;

impl Notifier for QuietNotifier {
    fn notify(&self, notification: Notification) {
        eprintln!("[通知] {}：{}", notification.title, notification.body);
    }
}

struct NoClipboard;

impl Clipboard for NoClipboard {
    fn write(&mut self, _items: Vec<ClipboardItem>) -> Result<(), PlatformError> {
        panic!("冒烟测试不应写剪贴板");
    }
}

fn input(instance: &mut ResidentInstance, input: OverlayInput) {
    instance
        .handle(PlatformEvent::OverlayInput(input))
        .expect("处理遮罩输入失败");
}

fn drag(instance: &mut ResidentInstance, from: Point, to: Point) {
    let none = Modifiers::NONE;
    input(
        instance,
        OverlayInput::PointerPressed {
            button: PointerButton::Left,
            position: from,
            modifiers: none,
            double_click: false,
        },
    );
    for step in 1..=8 {
        let at = Point::new(
            from.x + (to.x - from.x) * step / 8,
            from.y + (to.y - from.y) * step / 8,
        );
        input(
            instance,
            OverlayInput::PointerMoved {
                position: at,
                modifiers: none,
            },
        );
    }
    input(
        instance,
        OverlayInput::PointerReleased {
            button: PointerButton::Left,
            position: to,
            modifiers: none,
        },
    );
}

fn click(instance: &mut ResidentInstance, scene: &Rc<RefCell<Option<OverlayScene>>>, text: &str) {
    let rect = scene
        .borrow()
        .as_ref()
        .and_then(|s| s.text_rect(text))
        .unwrap_or_else(|| panic!("遮罩上没有“{text}”"));
    let at = Point::new(
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    );
    drag(instance, at, at);
}

/// 本进程里类名为 `class` 的可见窗口。
fn own_window(class: PCWSTR) -> Option<HWND> {
    let pid = unsafe { GetCurrentProcessId() };
    let mut after = None;
    loop {
        let hwnd = unsafe { FindWindowExW(None, after, class, None) }.ok()?;
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return Some(hwnd);
        }
        after = Some(hwnd);
    }
}

fn is_red(pixel: [u8; 4]) -> bool {
    let [r, g, b, _] = pixel;
    r >= 180 && g <= 100 && b <= 100
}

fn save_png(path: &Path, image: &Image) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(image.rgba()).unwrap();
}

/// `image`（左上角在虚拟桌面 `origin`）中 `area` 这一块。
fn crop(image: &Image, origin: Point, area: Rect) -> Image {
    let mut pixels = Vec::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let (sx, sy) = (area.x - origin.x + x as i32, area.y - origin.y + y as i32);
            pixels.extend_from_slice(&image.pixel(sx as u32, sy as u32));
        }
    }
    Image::from_rgba(area.width, area.height, pixels)
}

#[test]
#[ignore = "真机冒烟：在屏幕上显示选区遮罩并标注"]
fn annotation_on_real_overlay_matches_saved_screenshot() {
    assert!(dpi::ensure_per_monitor_v2());
    let shots = std::env::var_os("BPS_SMOKE_SHOTS").map(PathBuf::from);
    let temp = std::env::temp_dir().join(format!("bps-annotation-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let event_loop = EventLoop::new().unwrap();
    let mut platform = bps_platform_windows::platform(&event_loop, temp.join("config.toml"));
    let scene = Rc::new(RefCell::new(None));
    let real_overlay = std::mem::replace(
        &mut platform.overlay,
        Box::new(RecordingOverlay {
            inner: Box::new(NoOverlay),
            scene: scene.clone(),
        }),
    );
    platform.overlay = Box::new(RecordingOverlay {
        inner: real_overlay,
        scene: scene.clone(),
    });
    platform.notifier = Box::new(QuietNotifier);
    platform.clipboard = Box::new(NoClipboard);
    let mut instance = ResidentInstance::new(platform, Vec::new());

    let frames = DxgiFrameSource::new();
    let primary = frames
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let b = primary.bounds;
    let region = Rect::new(
        b.x + b.width as i32 / 4,
        b.y + b.height as i32 / 4,
        b.width / 2,
        b.height / 2,
    );
    let at = |fx: f64, fy: f64| {
        Point::new(
            region.x + (f64::from(region.width) * fx) as i32,
            region.y + (f64::from(region.height) * fy) as i32,
        )
    };
    let mut request = CaptureRequest::new(CaptureMode::Region, vec![PostCaptureAction::Annotate]);
    request.output.save_dir = SaveDir::Custom(temp.clone());

    let started = Instant::now();
    instance.execute(Command::Capture(request)).unwrap();
    drag(
        &mut instance,
        Point::new(region.x, region.y),
        Point::new(
            region.x + region.width as i32 - 1,
            region.y + region.height as i32 - 1,
        ),
    );
    let locked = Instant::now();
    click(&mut instance, &scene, "矩形");
    eprintln!("锁定区域并显示画布：{:?}", locked.elapsed());
    let rect_from = at(0.1, 0.1);
    let rect_to = at(0.6, 0.6);
    let drawing = Instant::now();
    drag(&mut instance, rect_from, rect_to);
    eprintln!("拖拽画框（10 次输入）：{:?}", drawing.elapsed());
    click(&mut instance, &scene, "箭头");
    let drawing = Instant::now();
    drag(&mut instance, at(0.9, 0.95), at(0.05, 0.05));
    eprintln!("拖拽画对角箭头（10 次输入）：{:?}", drawing.elapsed());

    // 屏幕上实际显示的画面。
    thread::sleep(Duration::from_millis(300));
    let screen = frames.capture_display(primary.id).unwrap();
    let screen_origin = Point::new(b.x, b.y);
    if let Some(dir) = &shots {
        let region_bottom = region.y + region.height as i32;
        let below = (b.y + b.height as i32 - region_bottom).clamp(0, 120);
        let toolbar_area = Rect::new(
            region.x + region.width as i32 / 2,
            region_bottom - 60,
            region.width / 2 + 10,
            (60 + below) as u32,
        );
        save_png(
            &dir.join("toolbar.png"),
            &crop(&screen, screen_origin, toolbar_area),
        );
    }

    // 另存为：对话框应以遮罩为所有者、置顶；在另一个线程里观察后关掉它（相当于取消）。
    let (sender, receiver) = mpsc::channel();
    let watcher = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(dialog) = own_window(w!("#32770")) {
                thread::sleep(Duration::from_millis(500));
                let owner = unsafe { GetWindow(dialog, GW_OWNER) }.ok();
                let overlay = own_window(w!("BetterPrtSc.SelectionOverlay"));
                let topmost =
                    unsafe { GetWindowLongW(dialog, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0;
                let shot = DxgiFrameSource::new().capture_display(primary.id).ok();
                unsafe { PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0)) }.unwrap();
                sender
                    .send((
                        owner.map(|h| h.0 as isize),
                        overlay.map(|h| h.0 as isize),
                        topmost,
                        shot,
                    ))
                    .unwrap();
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("10 秒内没有出现“另存为”对话框");
    });
    click(&mut instance, &scene, "另存为");
    let (owner, overlay, topmost, shot) = receiver.recv().unwrap();
    watcher.join().unwrap();
    eprintln!("另存为对话框：所有者 {owner:?}，遮罩 {overlay:?}，置顶 {topmost}");
    assert!(
        owner.is_some() && owner == overlay,
        "对话框的所有者应是遮罩窗口"
    );
    assert!(topmost, "对话框应置顶（在遮罩之上）");
    if let (Some(dir), Some(shot)) = (&shots, shot) {
        save_png(&dir.join("dialog.png"), &shot);
    }
    assert!(scene.borrow().is_some(), "取消另存为后应回到标注");

    click(&mut instance, &scene, "自动存储");
    eprintln!("从按下热键到存下截图：{:?}", started.elapsed());
    assert!(scene.borrow().is_none(), "自动存储后遮罩应关闭");

    let saved: Vec<PathBuf> = std::fs::read_dir(&temp)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    let image = bps_platform_fake::decode::png(&std::fs::read(&saved[0]).unwrap());
    assert_eq!(
        (image.width(), image.height()),
        (region.width, region.height)
    );
    if let Some(dir) = &shots {
        save_png(&dir.join("saved.png"), &image);
    }

    // 框的左上角线条：存下的截图和屏幕上都是红色；逐像素比较屏幕与存下的截图。
    let local = |p: Point| ((p.x - region.x) as u32, (p.y - region.y) as u32);
    let (fx, fy) = local(rect_from);
    assert!(is_red(image.pixel(fx + 1, fy + 1)), "存下的截图包含框");
    let shown = crop(&screen, screen_origin, region);
    let mut max_diff = 0;
    let mut differing = 0usize;
    for (a, b) in shown
        .rgba()
        .chunks_exact(4)
        .zip(image.rgba().chunks_exact(4))
    {
        let diff = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap();
        max_diff = max_diff.max(diff);
        if diff > 3 {
            differing += 1;
        }
    }
    eprintln!(
        "屏幕与存下的截图：最大通道差 {max_diff}，差值 > 3 的像素 {differing} / {}",
        region.width * region.height
    );
    assert!(is_red(shown.pixel(fx + 1, fy + 1)), "屏幕上显示着框");
    assert!(
        differing * 1000 < (region.width * region.height) as usize,
        "屏幕上看到的应与存下的截图一致"
    );
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
    fn close(&mut self) {}
}
