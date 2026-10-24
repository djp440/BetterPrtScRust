//! 选区遮罩真机冒烟：会在屏幕上闪现几次半透明遮罩（每次不到 1 秒），默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows --test overlay_smoke -- --ignored --nocapture`
//! 输入用 `PostMessage` 直接投递给遮罩窗口，不模拟真实键鼠。
#![cfg(windows)]

use std::time::{Duration, Instant};

use bps_platform::{
    Backdrop, Color, FrameSource, OverlayInput, OverlayItem, OverlayKey, OverlayScene,
    PlatformEvent, Point, PointerButton, SelectionOverlay,
};
use bps_platform_windows::{DxgiFrameSource, EventLoop, dpi};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, WM_KEYDOWN, WM_LBUTTONDOWN,
};
use windows::core::w;

/// 本进程的遮罩窗口（其他工作树的常驻实例也有同类名的隐藏窗口）。
fn own_overlay_window() -> HWND {
    let pid = unsafe { GetCurrentProcessId() };
    let mut after = None;
    loop {
        let hwnd = unsafe { FindWindowExW(None, after, w!("BetterPrtSc.SelectionOverlay"), None) }
            .expect("应已创建遮罩窗口");
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == pid {
            return hwnd;
        }
        after = Some(hwnd);
    }
}

fn dim_scene() -> OverlayScene {
    OverlayScene {
        items: vec![OverlayItem::Fill {
            rect: bps_platform::Rect::new(-100_000, -100_000, 200_000, 200_000),
            color: Color::rgba(0, 0, 0, 115),
        }],
        ..OverlayScene::default()
    }
}

#[test]
#[ignore = "真机冒烟：在屏幕上闪现选区遮罩"]
fn selection_overlay_shows_frozen_frame_and_delivers_input() {
    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let started = Instant::now();
    let mut overlay = event_loop.selection_overlay();
    eprintln!("创建并预热遮罩：{:?}", started.elapsed());
    let hwnd = own_overlay_window();
    assert!(
        !unsafe { IsWindowVisible(hwnd) }.as_bool(),
        "预热后遮罩应隐藏"
    );

    let expected = DxgiFrameSource::new().displays().unwrap();
    for round in 1..=3 {
        let started = Instant::now();
        let displays = overlay.open(Backdrop::FrozenFrame).unwrap();
        let frozen = started.elapsed();
        overlay.render(&dim_scene()).unwrap();
        eprintln!(
            "第 {round} 次：拍冻结帧 {frozen:?}，到显示完成 {:?}，前台={}",
            started.elapsed(),
            unsafe { GetForegroundWindow() } == hwnd
        );
        assert!(unsafe { IsWindowVisible(hwnd) }.as_bool(), "遮罩应已显示");
        let mut ids: Vec<_> = displays.iter().map(|d| (d.id, d.bounds)).collect();
        let mut want: Vec<_> = expected.iter().map(|d| (d.id, d.bounds)).collect();
        ids.sort_by_key(|(id, _)| id.0);
        want.sort_by_key(|(id, _)| id.0);
        assert_eq!(ids, want, "冻结帧应覆盖帧源报告的全部显示器");

        let mut window = Default::default();
        unsafe { GetWindowRect(hwnd, &mut window) }.unwrap();
        let origin = Point::new(window.left, window.top);
        unsafe {
            PostMessageW(
                Some(hwnd),
                WM_LBUTTONDOWN,
                WPARAM(0),
                LPARAM((20 << 16) | 10),
            )
            .unwrap();
            PostMessageW(
                Some(hwnd),
                WM_KEYDOWN,
                WPARAM(VK_ESCAPE.0 as usize),
                LPARAM(0),
            )
            .unwrap();
        }
        let pressed = event_loop.wait_event(Duration::from_secs(2));
        assert!(
            matches!(
                pressed,
                Some(PlatformEvent::OverlayInput(OverlayInput::PointerPressed {
                    button: PointerButton::Left,
                    position,
                    double_click: false,
                    ..
                })) if position == Point::new(origin.x + 10, origin.y + 20)
            ),
            "{pressed:?}"
        );
        let escape = event_loop.wait_event(Duration::from_secs(2));
        assert!(
            matches!(
                escape,
                Some(PlatformEvent::OverlayInput(OverlayInput::KeyPressed {
                    key: OverlayKey::Escape,
                    ..
                }))
            ),
            "{escape:?}"
        );

        for display in &displays {
            let image = overlay.frozen_image(display.id).unwrap();
            assert_eq!(
                (image.width(), image.height()),
                (display.bounds.width, display.bounds.height)
            );
            let opaque = image.rgba().chunks_exact(4).all(|p| p[3] == 255);
            assert!(opaque, "冻结帧应不透明");
        }
        overlay.close();
        assert!(
            !unsafe { IsWindowVisible(hwnd) }.as_bool(),
            "关闭后遮罩应隐藏"
        );
        assert!(
            event_loop.wait_event(Duration::from_millis(100)).is_none(),
            "关闭后不应再递交输入"
        );
    }
}
