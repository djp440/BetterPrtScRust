//! 真机冒烟测试：会真实截屏、注册全局热键、模拟按键、读写临时目录，默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows -- --ignored`
#![cfg(windows)]

use std::fs;
use std::time::{Duration, Instant};

use bps_platform::{
    Clock, FileSystem, FrameSource, GlobalHotkeys, Hotkey, HotkeyId, Key, Modifiers, NewFile,
    PlatformEvent,
};
use bps_platform_windows::{DxgiFrameSource, EventLoop, WindowsClock, WindowsFileSystem, dpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL, VK_F24,
};

#[test]
#[ignore = "真机冒烟：真实截取鼠标所在的显示器"]
fn captures_display_under_cursor_in_physical_pixels() {
    assert!(
        dpi::ensure_per_monitor_v2(),
        "进程应为 Per-Monitor V2 DPI 感知"
    );
    let frames = DxgiFrameSource::new();
    let displays = frames.displays().unwrap();
    assert!(!displays.is_empty(), "至少应有一块显示器");
    let cursor = frames.cursor_position().unwrap();
    let display = displays
        .iter()
        .find(|d| d.bounds.contains(cursor))
        .expect("鼠标应位于某块显示器内");

    let started = Instant::now();
    let image = frames.capture_display(display.id).unwrap();
    eprintln!(
        "显示器 {:?}（缩放 {}）截图 {}x{}，耗时 {:?}",
        display.bounds,
        display.scale_factor,
        image.width(),
        image.height(),
        started.elapsed()
    );

    assert_eq!(
        (image.width(), image.height()),
        (display.bounds.width, display.bounds.height),
        "截图应按物理像素输出"
    );
    let first = image.pixel(0, 0);
    assert!(
        image.rgba().chunks_exact(4).any(|p| p != first),
        "真实桌面截图不应是纯色"
    );
}

#[test]
#[ignore = "真机冒烟：注册全局热键 Ctrl+F24 并模拟按下"]
fn registered_hotkey_press_is_delivered_by_event_loop() {
    let event_loop = EventLoop::new().unwrap();
    let mut hotkeys = event_loop.hotkeys();
    let id = HotkeyId(7);
    // F24 几乎不会被占用；用 Ctrl 而不是 Shift/Alt，避免单独松开修饰键时切换输入法或激活菜单栏。
    hotkeys
        .register(id, Hotkey::new(Modifiers::CTRL, Key::F(24)))
        .unwrap();

    press_keys(&[VK_CONTROL, VK_F24]);
    let event = event_loop.wait_event(Duration::from_secs(2));
    hotkeys.unregister(id).unwrap();

    assert_eq!(event, Some(PlatformEvent::HotkeyPressed(id)));
}

#[test]
#[ignore = "真机冒烟：在临时目录里建目录、写文件，结束后删除该临时目录"]
fn file_system_creates_new_files_without_overwriting() {
    let mut fs_ = WindowsFileSystem::new();
    let pictures = fs_.pictures_dir().unwrap();
    eprintln!("“图片”文件夹：{}", pictures.display());
    assert!(pictures.is_absolute() && pictures.is_dir());

    let root = std::env::temp_dir().join(format!("bps-smoke-fs-{}", std::process::id()));
    let dir = root.join("子目录").join("BetterPrtSc");
    fs_.create_dir_all(&dir).unwrap();
    fs_.create_dir_all(&dir).unwrap(); // 已存在时不报错
    let file = dir.join("截图 2026-10-24.png");
    let first = fs_.create_new(&file, b"first");
    let second = fs_.create_new(&file, b"second");
    let content = fs::read(&file);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(first, Ok(NewFile::Created));
    assert_eq!(second, Ok(NewFile::AlreadyExists));
    assert_eq!(content.unwrap(), b"first");
}

#[test]
#[ignore = "真机冒烟：读取本地时间"]
fn clock_reports_plausible_local_time() {
    let now = WindowsClock::new().now();
    eprintln!("本地时间：{now:?}");
    assert!(now.year >= 2024);
    assert!((1..=12).contains(&now.month) && (1..=31).contains(&now.day));
    assert!(now.hour < 24 && now.minute < 60 && now.second < 61 && now.millisecond < 1000);
}

/// 依次按下 `keys`，再逆序松开。
fn press_keys(keys: &[VIRTUAL_KEY]) {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|vk| key(*vk, KEYBD_EVENT_FLAGS(0)))
        .chain(keys.iter().rev().map(|vk| key(*vk, KEYEVENTF_KEYUP)))
        .collect();
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    assert_eq!(
        sent as usize,
        inputs.len(),
        "模拟按键被拦截（前台可能是管理员窗口）"
    );
}
