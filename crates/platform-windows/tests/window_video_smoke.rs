//! 窗口录屏视频源的真机冒烟，默认忽略：
//! 临时显示两个本测试自己创建的置顶窗口（约 2 秒），下层窗口被上层窗口盖住右半边，
//! 检查视频源给出的画面仍是下层窗口的完整内容；再改变下层窗口的大小，检查画面尺寸不变、
//! 不出错；最后销毁下层窗口，检查流结束（`Ended`）。不碰其他程序的窗口、不启动常驻实例、
//! 不碰剪贴板。运行前要拿真机桌面锁：
//! `cargo test -p bps-platform-windows --test window_video_smoke -- --ignored --nocapture`
#![cfg(windows)]

use std::time::{Duration, Instant};

use bps_platform::{
    Image, StreamFrame, StreamOptions, VideoSource, VideoStream, VideoTarget, WindowId,
};
use bps_platform_windows::{WgcVideoSource, dpi};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SWP_NOMOVE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, TranslateMessage, WM_PAINT, WNDCLASSEXW, WS_EX_TOPMOST,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR, w};

const CLASS_NAME: PCWSTR = w!("BpsSmoke.WindowVideoTarget");
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

fn create_window(title: &str, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3]) -> HWND {
    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            CLASS_NAME,
            &HSTRING::from(title),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(module.into()),
            None,
        )
        .unwrap();
        let color = u32::from(rgb[0]) | u32::from(rgb[1]) << 8 | u32::from(rgb[2]) << 16;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, color as isize);
        hwnd
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

/// 泵消息并取流里的帧，直到 `done` 满足或超时。
fn poll(
    stream: &mut dyn VideoStream,
    timeout: Duration,
    done: impl Fn(&StreamFrame) -> bool,
) -> StreamFrame {
    let deadline = Instant::now() + timeout;
    loop {
        pump_messages(Duration::from_millis(30));
        let frame = stream.latest().unwrap();
        if done(&frame) || Instant::now() >= deadline {
            return frame;
        }
    }
}

fn near(pixel: [u8; 4], expected: [u8; 3]) -> bool {
    pixel[..3]
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(b) <= 6)
}

fn image_of(frame: &StreamFrame) -> Image {
    match frame {
        StreamFrame::Frame(frame) => frame.to_image().unwrap(),
        _ => panic!("应该有一帧画面"),
    }
}

#[test]
#[ignore = "真机冒烟：临时显示两个测试窗口并用 Windows.Graphics.Capture 录制下层窗口"]
fn records_covered_window_survives_resize_and_ends_when_closed() {
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
    let below = create_window("BetterPrtSc 窗口录屏冒烟 下层", 300, 300, 400, 300, RED);
    // 上层窗口盖住下层窗口的右半边以及更多。
    let above = create_window("BetterPrtSc 窗口录屏冒烟 上层", 500, 250, 500, 450, BLUE);
    pump_messages(Duration::from_millis(500));

    let (width, height) = (400, 300);
    let mut stream = WgcVideoSource::new()
        .open(
            VideoTarget::Window {
                window: WindowId(below.0 as usize as u64),
                width,
                height,
            },
            StreamOptions { show_cursor: false },
        )
        .unwrap();
    let first = poll(stream.as_mut(), Duration::from_secs(2), |f| {
        matches!(f, StreamFrame::Frame(_))
    });
    let image = image_of(&first);
    assert_eq!((image.width(), image.height()), (width, height));
    // 客户区（避开标题栏和边框）的左侧与被遮挡的右侧都应是下层窗口的红色。
    assert!(
        near(image.pixel(60, 150), RED),
        "左侧 {:?}",
        image.pixel(60, 150)
    );
    assert!(
        near(image.pixel(330, 150), RED),
        "被上层窗口遮挡的右侧 {:?}",
        image.pixel(330, 150)
    );
    eprintln!("被遮挡的窗口录到了完整内容");

    // 窗口变大、再变小：画面尺寸不变，流不出错。
    for (w, h) in [(600, 420), (250, 200)] {
        unsafe {
            SetWindowPos(below, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER).unwrap();
        }
        let frame = poll(stream.as_mut(), Duration::from_millis(600), |_| false);
        let image = image_of(&frame);
        assert_eq!((image.width(), image.height()), (width, height));
        eprintln!("窗口改为 {w}x{h} 后画面仍为 {width}x{height}");
    }

    // 窗口关闭：流结束。
    unsafe {
        DestroyWindow(below).unwrap();
    }
    let ended = poll(stream.as_mut(), Duration::from_secs(2), |f| {
        matches!(f, StreamFrame::Ended)
    });
    assert!(matches!(ended, StreamFrame::Ended), "窗口关闭后流应结束");
    eprintln!("窗口关闭后流结束");
    drop(stream);
    unsafe {
        let _ = DestroyWindow(above);
    }
}
