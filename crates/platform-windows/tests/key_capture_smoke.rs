//! 真机冒烟测试：录制热键用的低层键盘钩子，默认忽略。
//!
//! 运行（需持桌面锁）：`cargo test -p bps-platform-windows --test key_capture_smoke -- --ignored`
//! 钩子期间模拟按下 F24（没有实体键盘有这个键，不会影响正在使用的程序），确认事件送达并被吞掉，
//! 丢弃守卫后钩子卸载（再次可以开始录制）。
#![cfg(windows)]

use std::time::Duration;

use bps_platform_windows::start_key_capture;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_F24, keybd_event,
};

#[test]
#[ignore = "真机冒烟：安装低层键盘钩子并模拟按键"]
fn hook_delivers_key_events_and_uninstalls_when_dropped() {
    let (inputs, guard) = start_key_capture().unwrap();
    assert!(start_key_capture().is_err(), "同一时刻只允许一个录制");

    unsafe {
        keybd_event(VK_F24.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_F24.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
    let down = inputs.recv_timeout(Duration::from_secs(3)).unwrap();
    let up = inputs.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!((down.vk, down.pressed), (VK_F24.0 as u32, true));
    assert_eq!((up.vk, up.pressed), (VK_F24.0 as u32, false));

    drop(guard);
    let (_inputs, guard) = start_key_capture().expect("卸载后可以再次开始");
    drop(guard);
}
