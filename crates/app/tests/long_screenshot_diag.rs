//! 长截屏真机诊断，默认忽略：用独立配置目录打开一个 Edge 窗口显示一张长网页，按慢速滚轮
//! 向下滚动，把 Windows.Graphics.Capture 的真实采样帧逐帧交给 [`LongScreenshot`]，统计
//! 拼接结果；丢帧（`Lost`）时把参照帧和这一帧存成 PNG 供排查。第一帧和常驻实例一样取 DXGI
//! 冻结帧。不启动常驻实例、不碰剪贴板。运行期间请勿操作电脑：
//! `cargo test -p bps-app --test long_screenshot_diag -- --ignored --nocapture`
//!
//! `BPS_DIAG_FULL=1` 时区域取整个主显示器（模拟单击选中最大化窗口），否则取主显示器中间一半。
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use bps_core::{LongScreenshot, Stitched};
use bps_platform::{FrameSource, Image, PlatformEvent, Rect, RegionSampler};
use bps_platform_windows::{DxgiFrameSource, EventLoop, WgcRegionSampler, dpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

const NOTCHES: u32 = 20;
const NOTCH_INTERVAL: Duration = Duration::from_millis(500);

#[test]
#[ignore = "真机诊断：Edge 慢速滚动的长截屏拼接"]
fn slow_wheel_scrolling_in_edge_is_stitched() {
    assert!(dpi::ensure_per_monitor_v2());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/long-diag");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let page = root.join("page.html");
    std::fs::write(&page, long_page()).unwrap();

    let mut edge = Command::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
        .arg(format!(
            "--user-data-dir={}",
            root.join("profile").display()
        ))
        .args([
            "--no-first-run",
            "--disable-fre",
            "--no-default-browser-check",
            "--inprivate",
            "--new-window",
            "--start-maximized",
        ])
        .arg(format!(
            "file:///{}",
            page.display().to_string().replace('\\', "/")
        ))
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_secs(5));

    let result = std::panic::catch_unwind(|| run(&root));
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &edge.id().to_string()])
        .output();
    let _ = edge.wait();
    let lost = result.unwrap();
    assert_eq!(lost, 0, "慢速滚动不应丢帧，见 {}", root.display());
}

fn run(root: &Path) -> u32 {
    let event_loop = EventLoop::new().unwrap();
    let frames = DxgiFrameSource::new();
    let primary = frames
        .displays()
        .unwrap()
        .into_iter()
        .find(|d| d.is_primary)
        .unwrap();
    let b = primary.bounds;
    let region = if std::env::var_os("BPS_DIAG_FULL").is_some() {
        b
    } else {
        Rect::new(
            b.x + b.width as i32 / 4,
            b.y + b.height as i32 / 4,
            b.width / 2,
            b.height / 2,
        )
    };
    eprintln!(
        "显示器 {b:?}，缩放 {}，区域 {region:?}",
        primary.scale_factor
    );
    unsafe {
        SetCursorPos(
            region.x + region.width as i32 / 2,
            region.y + region.height as i32 / 2,
        )
        .unwrap();
    }
    thread::sleep(Duration::from_millis(300));

    let desktop = frames.capture_display(primary.id).unwrap();
    let first = desktop
        .crop(Rect::new(
            region.x - b.x,
            region.y - b.y,
            region.width,
            region.height,
        ))
        .unwrap();
    let mut sampler = WgcRegionSampler::new(event_loop.ticker());
    sampler.start(region).unwrap();
    let mut long = LongScreenshot::new(first.clone());
    let mut reference = first.clone();
    let mut compared_first = false;
    let (mut lost, mut scrolled, mut unchanged, mut samples) = (0u32, 0u32, 0u32, 0u32);
    let started = Instant::now();
    let mut next_notch = started + Duration::from_millis(500);
    let mut notches = 0;
    let end = started + NOTCH_INTERVAL * (NOTCHES + 2);
    while Instant::now() < end {
        if notches < NOTCHES && Instant::now() >= next_notch {
            wheel(-120);
            notches += 1;
            next_notch += NOTCH_INTERVAL;
        }
        let Some(PlatformEvent::Tick) = event_loop.wait_event(Duration::from_millis(10)) else {
            continue;
        };
        let Some(frame) = sampler.take_sample().unwrap() else {
            continue;
        };
        samples += 1;
        if !compared_first {
            compared_first = true;
            let differing = differing_pixels(&first, &frame);
            eprintln!("首个 WGC 采样与 DXGI 冻结帧不同的像素：{differing}");
            if differing > 0 {
                save(&root.join("first-dxgi.png"), &first);
                save(&root.join("first-wgc.png"), &frame);
            }
        }
        let t = started.elapsed().as_millis();
        match long.push(&frame) {
            Stitched::Scrolled { distance, added } => {
                scrolled += 1;
                eprintln!(
                    "{t:>6} ms  滚动 {distance:>5}  新增 {added:>5}  高度 {}",
                    long.height()
                );
                reference = frame;
            }
            Stitched::Unchanged => unchanged += 1,
            Stitched::Lost => {
                lost += 1;
                eprintln!("{t:>6} ms  丢帧 #{lost}");
                if lost <= 5 {
                    save(&root.join(format!("lost-{lost}-reference.png")), &reference);
                    save(&root.join(format!("lost-{lost}-frame.png")), &frame);
                }
            }
            Stitched::Full => break,
        }
    }
    sampler.stop();
    save(&root.join("long.png"), &long.image());
    eprintln!(
        "采样 {samples}，滚动 {scrolled}，未变 {unchanged}，丢帧 {lost}，长图高度 {}",
        long.height()
    );
    lost
}

fn wheel(delta: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: delta as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                ..Default::default()
            },
        },
    };
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

fn differing_pixels(a: &Image, b: &Image) -> usize {
    if (a.width(), a.height()) != (b.width(), b.height()) {
        return usize::MAX;
    }
    a.rgba()
        .chunks_exact(4)
        .zip(b.rgba().chunks_exact(4))
        .filter(|(p, q)| p != q)
        .count()
}

fn save(path: &PathBuf, image: &Image) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(image.rgba()).unwrap();
}

/// 一张有吸顶栏、内容各不相同的长网页。
fn long_page() -> String {
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><style>\
         body{font:16px sans-serif;margin:0}\
         header{position:sticky;top:0;background:#246;color:#fff;padding:12px}\
         p{margin:8px 24px}</style><header>长截屏诊断页</header>",
    );
    for i in 0..400 {
        html.push_str(&format!(
            "<p>第 {i} 段：{}</p>",
            "滚动拼接测试文字 ".repeat(1 + i % 7)
        ));
    }
    html
}

#[test]
#[ignore = "离线重放 logs/long-diag 里保存的丢帧"]
fn replay_saved_lost_frames() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/long-diag");
    for i in 1..=5 {
        let (r, f) = (
            root.join(format!("lost-{i}-reference.png")),
            root.join(format!("lost-{i}-frame.png")),
        );
        if !r.exists() {
            break;
        }
        let mut long = LongScreenshot::new(load(&r));
        eprintln!("重放 #{i}：{:?}", long.push(&load(&f)));
    }
}

fn load(path: &Path) -> Image {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    Image::from_rgba(info.width, info.height, buf)
}
