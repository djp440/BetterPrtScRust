//! 长截屏真机冒烟，默认忽略：冻结帧遮罩挖出一块“透过”的区域，区域采样应采到底下的实时桌面
//! （而不是被压暗的遮罩），遮罩窗口不出现在采样里；不启动常驻实例、不碰剪贴板、不注册热键。
//! 会闪现遮罩约 2 秒，运行前要拿真机桌面锁：
//! `cargo test -p bps-platform-windows --test long_screenshot_smoke -- --ignored --nocapture --test-threads=1`
#![cfg(windows)]

use std::time::{Duration, Instant};

use bps_platform::{
    Backdrop, Color, FrameSource, OverlayItem, OverlayScene, PlatformEvent, Rect, RegionSampler,
    SelectionOverlay,
};
use bps_platform_windows::{DxgiFrameSource, EventLoop, WgcRegionSampler, dpi};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};
use windows::core::w;

fn mean_brightness(image: &bps_platform::Image) -> f64 {
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in (0..image.height()).step_by(4) {
        for x in (0..image.width()).step_by(4) {
            let p = image.pixel(x, y);
            sum += u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]);
            count += 3;
        }
    }
    sum as f64 / count as f64
}

#[test]
#[ignore = "真机冒烟：遮罩透过 + 区域采样"]
fn pass_through_region_is_sampled_live_and_mask_is_excluded() {
    assert!(dpi::ensure_per_monitor_v2());
    let event_loop = EventLoop::new().unwrap();
    let mut overlay = event_loop.selection_overlay();
    let frames = DxgiFrameSource::new();
    let primary = frames
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let region = Rect::new(primary.bounds.x + 100, primary.bounds.y + 100, 320, 240);

    overlay.open(Backdrop::FrozenFrame).unwrap();
    overlay
        .render(&OverlayScene {
            items: vec![OverlayItem::Fill {
                rect: primary.bounds,
                color: Color::rgba(0, 0, 0, 230),
            }],
            ..OverlayScene::default()
        })
        .unwrap();
    overlay.set_pass_through(Some(region)).unwrap();

    let hwnd = unsafe { FindWindowExW(None, None, w!("BetterPrtSc.SelectionOverlay"), None) };
    if let Ok(hwnd) = hwnd {
        let mut affinity = 0;
        unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.unwrap();
        assert_eq!(
            affinity, WDA_EXCLUDEFROMCAPTURE.0,
            "透过期间遮罩应排除在捕获之外"
        );
    }

    let mut sampler = WgcRegionSampler::new(event_loop.ticker());
    sampler.start(region).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut sample = None;
    while sample.is_none() && Instant::now() < deadline {
        if let Some(PlatformEvent::Tick) = event_loop.wait_event(Duration::from_millis(100)) {
            sample = sampler.take_sample().unwrap();
        }
    }
    sampler.stop();
    let sample = sample.expect("3 秒内应采到一帧");
    assert_eq!(
        (sample.width(), sample.height()),
        (region.width, region.height)
    );

    let live = frames.capture_display(primary.id).unwrap();
    overlay.set_pass_through(None).unwrap();
    overlay.close();
    // 遮罩压暗到 10% 左右；采到的应与不含遮罩的实时桌面一样亮（容许桌面自己在变化）。
    let (rx, ry) = (
        (region.x - primary.bounds.x) as u32,
        (region.y - primary.bounds.y) as u32,
    );
    let mut pixels = Vec::new();
    for y in 0..region.height {
        for x in 0..region.width {
            pixels.extend(live.pixel(rx + x, ry + y));
        }
    }
    let desk = bps_platform::Image::from_rgba(region.width, region.height, pixels);
    let (a, b) = (mean_brightness(&sample), mean_brightness(&desk));
    eprintln!("采样亮度 {a:.1}，桌面亮度 {b:.1}");
    assert!(
        (a - b).abs() <= 25.0 + b * 0.1,
        "采样应是实时桌面而不是压暗的遮罩：{a} vs {b}"
    );
    let _ = WDA_NONE;
}
