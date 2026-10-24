//! 窗口枚举与 Windows.Graphics.Capture 窗口截图的真机冒烟，默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows --test window_smoke -- --ignored --nocapture`
//! 会在屏幕上临时显示两个本测试自己创建的置顶窗口（约 1 秒，其中一个会短暂最大化），结束时销毁；
//! 不碰其他程序的窗口。`print_current_window_layout` 只打印当前窗口布局，供排查用。
#![cfg(windows)]

use std::time::{Duration, Instant};

use bps_platform::{Image, Rect, WindowSource};
use bps_platform_windows::{WgcWindowSource, dpi};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, EndPaint, FillRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, GetWindowRect, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SW_MAXIMIZE,
    SetWindowLongPtrW, ShowWindow, TranslateMessage, UnregisterClassW, WM_PAINT, WNDCLASSEXW,
    WS_CHILD, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR, w};

const CLASS_NAME: PCWSTR = w!("BpsSmoke.WindowCaptureTarget");
const RED: [u8; 3] = [200, 40, 40];
const BLUE: [u8; 3] = [40, 40, 200];

unsafe extern "system" fn paint_solid(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_PAINT {
        unsafe {
            let color = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32;
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let brush = CreateSolidBrush(COLORREF(color));
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            FillRect(dc, &client, brush);
            let _ = DeleteObject(brush.into());
            let _ = EndPaint(hwnd, &paint);
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 本测试自己创建的置顶窗口，客户区涂成纯色；释放时销毁。
struct TestWindow(HWND);

impl TestWindow {
    fn new(title: &str, outer: Rect, rgb: [u8; 3]) -> Self {
        unsafe {
            let module = GetModuleHandleW(None).unwrap();
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                CLASS_NAME,
                &HSTRING::from(title),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                outer.x,
                outer.y,
                outer.width as i32,
                outer.height as i32,
                None,
                None,
                Some(module.into()),
                None,
            )
            .unwrap();
            let color = u32::from(rgb[0]) | u32::from(rgb[1]) << 8 | u32::from(rgb[2]) << 16;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, color as isize);
            Self(hwnd)
        }
    }

    /// 客户区 `(x, y)` 处的点在屏幕上的位置。
    fn client_to_screen(&self, x: i32, y: i32) -> (i32, i32) {
        let mut point = POINT { x, y };
        unsafe {
            let _ = ClientToScreen(self.0, &mut point);
        }
        (point.x, point.y)
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

fn pump_messages(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn rgb_near(pixel: [u8; 4], expected: [u8; 3]) -> bool {
    pixel[..3]
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(b) <= 6)
}

/// 图像中 `(x, y)` 处的像素，`(x, y)` 是屏幕坐标、`origin` 是图像左上角在屏幕上的位置。
fn pixel_at(image: &Image, origin: Rect, (x, y): (i32, i32)) -> [u8; 4] {
    image.pixel((x - origin.x) as u32, (y - origin.y) as u32)
}

#[test]
#[ignore = "真机冒烟：临时显示两个测试窗口并用 Windows.Graphics.Capture 截取"]
fn window_layout_hit_test_and_capture_of_partly_covered_window() {
    assert!(dpi::ensure_per_monitor_v2());
    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(paint_solid),
            hInstance: module.into(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        assert_ne!(RegisterClassExW(&class), 0);
    }
    let below = TestWindow::new("BetterPrtSc 冒烟 下层", Rect::new(300, 300, 500, 400), RED);
    let control = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("控件"),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            120,
            40,
            Some(below.0),
            None,
            None,
            None,
        )
    }
    .unwrap();
    let above = TestWindow::new("BetterPrtSc 冒烟 上层", Rect::new(550, 500, 500, 400), BLUE);
    pump_messages(Duration::from_millis(600));

    let source = WgcWindowSource::new();
    let started = Instant::now();
    let layout = source.layout().unwrap();
    for window in layout.windows() {
        eprintln!("  {:?} {:?}", window.title, window.bounds);
    }
    eprintln!(
        "枚举顶层窗口：{:?}，共 {} 个",
        started.elapsed(),
        layout.windows().len()
    );
    let position = |title: &str| {
        layout
            .windows()
            .iter()
            .position(|w| w.title == title)
            .unwrap_or_else(|| panic!("布局里应有“{title}”：{:#?}", layout.windows()))
    };
    let (below_index, above_index) = (
        position("BetterPrtSc 冒烟 下层"),
        position("BetterPrtSc 冒烟 上层"),
    );
    assert!(above_index < below_index, "后显示的置顶窗口应在上层");
    let below_info = layout.windows()[below_index].clone();
    let above_info = layout.windows()[above_index].clone();
    let mut outer = RECT::default();
    unsafe { GetWindowRect(below.0, &mut outer) }.unwrap();
    eprintln!(
        "下层窗口：外框 {:?}，GetWindowRect {:?}",
        below_info.bounds, outer
    );
    assert!(
        !layout
            .windows()
            .iter()
            .any(|w| w.title.contains("选区遮罩")),
        "不应包含 BetterPrtSc 自己的窗口"
    );

    // 命中测试：重叠处是上层窗口，只露出下层窗口的地方是下层窗口；不深入到控件。
    let overlap = above.client_to_screen(10, 10);
    let only_below = below.client_to_screen(200, 150);
    let hit = |(x, y): (i32, i32)| {
        layout
            .window_at(bps_platform::Point::new(x, y))
            .map(|w| w.title.clone())
    };
    assert_eq!(hit(overlap).as_deref(), Some("BetterPrtSc 冒烟 上层"));
    assert_eq!(hit(only_below).as_deref(), Some("BetterPrtSc 冒烟 下层"));

    let controls = source.controls(below_info.id).unwrap();
    let mut control_rect = RECT::default();
    unsafe { GetWindowRect(control, &mut control_rect) }.unwrap();
    assert_eq!(
        controls,
        [Rect::new(
            control_rect.left,
            control_rect.top,
            (control_rect.right - control_rect.left) as u32,
            (control_rect.bottom - control_rect.top) as u32
        )]
    );

    let started = Instant::now();
    let image = source.capture_window(below_info.id).unwrap();
    eprintln!(
        "截取窗口：{:?}，图像 {}x{}",
        started.elapsed(),
        image.width(),
        image.height()
    );
    let corner_alpha: Vec<u8> = (0..8).map(|i| image.pixel(i, i)[3]).collect();
    eprintln!("左上角对角线 alpha：{corner_alpha:?}");
    let right = image.width() - 1;
    let bottom = image.height() - 1;
    eprintln!(
        "四角像素：{:?} {:?} {:?} {:?}",
        image.pixel(0, 0),
        image.pixel(right, 0),
        image.pixel(0, bottom),
        image.pixel(right, bottom)
    );
    let translucent = image.rgba().chunks_exact(4).filter(|p| p[3] < 255).count();
    eprintln!("alpha < 255 的像素数：{translucent}");
    for y in 0..8 {
        let row: Vec<u8> = (0..20).map(|x| image.pixel(x, y)[3]).collect();
        eprintln!("第 {y} 行前 20 个 alpha：{row:?}");
    }
    let middle = image.width() / 2;
    eprintln!(
        "上边中间往下 4 个像素：{:?}",
        (0..4).map(|y| image.pixel(middle, y)).collect::<Vec<_>>()
    );
    eprintln!(
        "左边中间往右 4 个像素：{:?}",
        (0..4)
            .map(|x| image.pixel(x, image.height() / 2))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (image.width(), image.height()),
        (below_info.bounds.width, below_info.bounds.height),
        "窗口截图应与窗口外框（不含阴影）一样大"
    );
    for (x, y) in [(0, 0), (right, 0), (0, bottom), (right, bottom)] {
        let corner = image.pixel(x, y);
        assert!(
            corner == [0, 0, 0, 0] || corner[3] == 255,
            "圆角以外应完全透明（没有圆角时是不透明的窗口边缘）：({x}, {y}) {corner:?}"
        );
    }
    let visible = pixel_at(&image, below_info.bounds, only_below);
    assert!(rgb_near(visible, RED), "露出的部分应是红色：{visible:?}");
    let covered = pixel_at(&image, below_info.bounds, below.client_to_screen(400, 300));
    assert!(
        rgb_near(covered, RED),
        "被上层窗口挡住的部分也应是窗口自己的红色：{covered:?}"
    );
    assert_eq!(visible[3], 255);

    let image = source.capture_window(above_info.id).unwrap();
    let center = pixel_at(&image, above_info.bounds, above.client_to_screen(100, 100));
    assert!(rgb_near(center, BLUE), "{center:?}");

    // 最大化的窗口：不可见的缩放边框在显示器外面，截图仍应与可见外框一样大。
    unsafe {
        let _ = ShowWindow(above.0, SW_MAXIMIZE);
    }
    pump_messages(Duration::from_millis(500));
    let maximized = source
        .layout()
        .unwrap()
        .get(above_info.id)
        .cloned()
        .expect("最大化后仍在布局里");
    let image = source.capture_window(above_info.id).unwrap();
    eprintln!(
        "最大化：外框 {:?}，截图 {}x{}，左上角 {:?}",
        maximized.bounds,
        image.width(),
        image.height(),
        image.pixel(0, 0)
    );
    assert_eq!(
        (image.width(), image.height()),
        (maximized.bounds.width, maximized.bounds.height)
    );

    let id = below_info.id;
    drop(below);
    drop(above);
    assert!(source.capture_window(id).is_err(), "已关闭的窗口应截取失败");
    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let _ = UnregisterClassW(CLASS_NAME, Some(module.into()));
    }
}

/// 诊断用：打印此刻的顶层窗口布局（标题与外框），不显示也不操作任何窗口。
#[test]
#[ignore = "诊断：打印此刻桌面上的顶层窗口布局"]
fn print_current_window_layout() {
    assert!(dpi::ensure_per_monitor_v2());
    let layout = WgcWindowSource::new().layout().unwrap();
    for window in layout.windows() {
        eprintln!("{:?} {:?} {:?}", window.id, window.title, window.bounds);
    }
}
