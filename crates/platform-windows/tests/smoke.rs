//! 真机冒烟测试：会真实截屏、注册全局热键并模拟按键，默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows -- --ignored`
#![cfg(windows)]

use std::time::{Duration, Instant};

use bps_platform::{
    ChannelError, FrameSource, GlobalHotkeys, Hotkey, HotkeyId, Key, Modifiers, PlatformEvent,
    ResidentChannel,
};
use bps_platform_windows::{DxgiFrameSource, EventLoop, PipeChannel, PipeClaim, ResidentPipe, dpi};
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

    assert!(
        matches!(event, Some(PlatformEvent::HotkeyPressed(pressed)) if pressed == id),
        "{event:?}"
    );
}

#[test]
#[ignore = "真机冒烟：创建命名管道并经它转交一条请求"]
fn resident_pipe_is_single_instance_and_forwards_requests_to_event_loop() {
    let name = format!(r"\\.\pipe\BetterPrtSc-smoke-{}", std::process::id());
    let PipeClaim::Claimed(pipe) = ResidentPipe::claim(&name).unwrap() else {
        panic!("第一次占用管道应成功");
    };
    assert!(
        matches!(ResidentPipe::claim(&name), Ok(PipeClaim::AlreadyRunning)),
        "管道已被占用时不应再成为常驻实例"
    );
    let event_loop = EventLoop::new().unwrap();
    pipe.serve(event_loop.sender()).unwrap();

    let client_name = name.clone();
    let client = std::thread::spawn(move || {
        // 常驻实例在运行，所以永远不会拉起这个不存在的程序。
        PipeChannel::new(&client_name, "不存在的程序.exe").send("capture display")
    });
    let Some(PlatformEvent::RequestReceived(request)) =
        event_loop.wait_event(Duration::from_secs(5))
    else {
        panic!("事件循环应收到转交来的请求");
    };
    assert_eq!(request.message(), "capture display");
    request.reply("ok".to_owned());

    assert_eq!(client.join().unwrap(), Ok("ok".to_owned()));
    let mut gone = PipeChannel::new(r"\\.\pipe\BetterPrtSc-smoke-nobody", "不存在的程序.exe");
    assert_eq!(gone.send("activate"), Err(ChannelError::NotRunning));
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
