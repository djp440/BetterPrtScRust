//! 窗口枚举与命中测试能力：`EnumWindows` 给出顶层窗口布局，`EnumChildWindows` 给出控件，
//! Windows.Graphics.Capture 截取单个窗口（见 [`crate::window_capture`]）。

use std::time::Duration;

use bps_platform::{
    Image, PlatformError, Rect, TopLevelWindow, WindowId, WindowLayout, WindowSource,
};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GWL_EXSTYLE, GetClassNameW, GetLayeredWindowAttributes,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow,
    IsWindowVisible, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};
use windows::core::BOOL;

use crate::window_capture::{BgraFrame, WindowCaptureSession};

/// 等待第一帧窗口画面的最长时间。这是在事件循环线程上同步等待的，所以要短：正常情况下第一帧
/// 几十毫秒内就到，等到超时说明窗口不可捕获（最小化、正在关闭、不允许捕获），此时捕获失败并通知用户。
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_millis(400);
/// 圆角最多可能占的范围（物理像素）：Win11 的圆角半径是 8 DIP，800% 缩放下 64 像素。
const CORNER_ZONE: u32 = 64;
/// 圆角以外残留的阴影最多这么不透明（实测 Win11 在 7–36 之间，窗口描边约 110）。
const CORNER_RESIDUE_ALPHA: u8 = 64;

/// 窗口枚举与命中测试的 Windows 实现。`WindowId` 是窗口的 `HWND` 值。
#[derive(Debug, Default)]
pub struct WgcWindowSource;

impl WgcWindowSource {
    pub fn new() -> Self {
        Self
    }
}

impl WindowSource for WgcWindowSource {
    fn layout(&self) -> Result<WindowLayout, PlatformError> {
        let mut handles: Vec<HWND> = Vec::new();
        unsafe {
            EnumWindows(
                Some(collect_window),
                LPARAM(&mut handles as *mut Vec<HWND> as isize),
            )
        }
        .map_err(|e| crate::error::win_error("枚举窗口失败", e))?;
        // EnumWindows 按 Z 序从上到下给出顶层窗口。
        let windows = handles
            .into_iter()
            .filter(|&hwnd| is_user_visible(hwnd))
            .filter_map(|hwnd| {
                let bounds = frame_bounds(hwnd)?;
                Some(TopLevelWindow {
                    id: id_of(hwnd),
                    title: title_of(hwnd),
                    bounds,
                })
            })
            .collect();
        Ok(WindowLayout::new(windows))
    }

    fn controls(&self, window: WindowId) -> Result<Vec<Rect>, PlatformError> {
        let hwnd = hwnd_of(window)?;
        let mut handles: Vec<HWND> = Vec::new();
        unsafe {
            // 回调总是返回 TRUE，EnumChildWindows 的返回值没有意义。
            let _ = EnumChildWindows(
                Some(hwnd),
                Some(collect_window),
                LPARAM(&mut handles as *mut Vec<HWND> as isize),
            );
        }
        Ok(handles
            .into_iter()
            .filter(|&child| unsafe { IsWindowVisible(child) }.as_bool())
            .filter_map(window_rect)
            .collect())
    }

    fn capture_window(&self, window: WindowId) -> Result<Image, PlatformError> {
        let hwnd = hwnd_of(window)?;
        if unsafe { IsIconic(hwnd) }.as_bool() {
            return Err(PlatformError::new("窗口已最小化，无法截取"));
        }
        let session = WindowCaptureSession::start(hwnd)?;
        let frame = session.next_frame(FIRST_FRAME_TIMEOUT)?;
        let bgra = session.read_back(&frame)?;
        let mut image = to_rgba(bgra);
        clear_outside_rounded_corners(&mut image);
        Ok(Image::from_rgba(image.width, image.height, image.pixels))
    }
}

/// 非预乘 RGBA 像素，原地修改用。
struct RgbaFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// 把 WGC 读回的预乘 BGRA 转换成非预乘 RGBA。
fn to_rgba(frame: BgraFrame) -> RgbaFrame {
    let mut rgba = frame.pixels;
    for p in rgba.chunks_exact_mut(4) {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        let unpremultiply = |c: u8| match a {
            0 => 0,
            255 => c,
            a => ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8,
        };
        p.copy_from_slice(&[unpremultiply(r), unpremultiply(g), unpremultiply(b), a]);
    }
    RgbaFrame {
        width: frame.width,
        height: frame.height,
        pixels: rgba,
    }
}

/// 让圆角以外的像素完全透明。
///
/// Win11 上 WGC 截到的窗口带圆角，但圆角以外不是完全透明的，而是残留着很淡的黑色阴影
/// （alpha 约 7–36）；不处理的话 JPG 合成到白底后四个角会发灰。做法：在四个角的范围内逐行
/// 从外边缘往里扫，把 alpha 低于 [`CORNER_RESIDUE_ALPHA`] 的像素清成透明，遇到圆角描边或
/// 窗口内容（alpha 更高）就停。没有圆角的窗口（Win10、最大化）第一个像素就不透明，不受影响。
fn clear_outside_rounded_corners(image: &mut RgbaFrame) {
    let (width, height) = (image.width as usize, image.height as usize);
    let zone_w = (image.width / 2).min(CORNER_ZONE) as usize;
    let zone_h = (image.height / 2).min(CORNER_ZONE) as usize;
    let rows = (0..zone_h).chain(height - zone_h..height);
    for y in rows {
        let row = &mut image.pixels[y * width * 4..(y + 1) * width * 4];
        let mut clear = |x: usize| {
            let pixel = &mut row[x * 4..x * 4 + 4];
            if pixel[3] >= CORNER_RESIDUE_ALPHA {
                return false;
            }
            pixel.copy_from_slice(&[0, 0, 0, 0]);
            true
        };
        for x in 0..zone_w {
            if !clear(x) {
                break;
            }
        }
        for x in (width - zone_w..width).rev() {
            if !clear(x) {
                break;
            }
        }
    }
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let handles = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    handles.push(hwnd);
    true.into()
}

/// 用户在屏幕上看得见、可以选中的顶层窗口：可见、未最小化、没有被系统隐藏（cloaked）、
/// 鼠标不会穿透、不完全透明，不是 BetterPrtSc 自己的窗口（选区遮罩等），也不是桌面本身
/// （`Progman` / `WorkerW`：壁纸和图标，铺满整个虚拟桌面）。
fn is_user_visible(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut cloaked = 0u32;
        let cloaked_ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
        .is_ok();
        if cloaked_ok && cloaked != 0 {
            return false;
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TRANSPARENT.0 != 0 {
            return false;
        }
        if ex_style & WS_EX_LAYERED.0 != 0 {
            let mut alpha = 0u8;
            let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();
            if GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)).is_ok()
                && flags.contains(LWA_ALPHA)
                && alpha == 0
            {
                return false;
            }
        }
        let class = class_name(hwnd);
        !(class.starts_with(OWN_CLASS_PREFIX) || class == "Progman" || class == "WorkerW")
    }
}

/// BetterPrtSc 所有窗口类名的前缀（如 `BetterPrtSc.SelectionOverlay`）。按类名而不是按进程排除，
/// 别的进程（如另一份正在运行的 BetterPrtSc）的遮罩也不会被当成可选的窗口。
const OWN_CLASS_PREFIX: &str = "BetterPrtSc.";

fn class_name(hwnd: HWND) -> String {
    let mut class = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    String::from_utf16_lossy(&class[..len])
}

/// 窗口在屏幕上可见部分的外框（DWM 扩展边框，不含阴影和不可见的缩放边框）。
fn frame_bounds(hwnd: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    }
    .is_ok();
    if ok { to_rect(rect) } else { window_rect(hwnd) }
}

fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    to_rect(rect)
}

/// 空矩形返回 `None`。
fn to_rect(rect: RECT) -> Option<Rect> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    (width > 0 && height > 0).then(|| Rect::new(rect.left, rect.top, width as u32, height as u32))
}

fn title_of(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer) as usize;
        String::from_utf16_lossy(&buffer[..copied.min(buffer.len())])
    }
}

fn id_of(hwnd: HWND) -> WindowId {
    WindowId(hwnd.0 as usize as u64)
}

/// 标识对应的窗口；窗口已经关闭时报错。
pub(crate) fn hwnd_of(window: WindowId) -> Result<HWND, PlatformError> {
    let hwnd = HWND(window.0 as usize as *mut _);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Ok(hwnd)
    } else {
        Err(PlatformError::new("窗口已关闭"))
    }
}
