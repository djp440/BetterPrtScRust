//! 截屏能力：`EnumDisplayMonitors` 给出显示器布局，DXGI Desktop Duplication 截图（ADR 0002）。

use bps_platform::{Display, DisplayId, FrameSource, Image, PlatformError, Point, Rect};
use windows::Win32::Foundation::{HMODULE, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTPUT_DESC, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIOutput5,
    IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, MONITORINFOF_PRIMARY};
use windows::core::{BOOL, Interface};

use crate::display_config::sdr_white_level;
use crate::error::win_error;
use crate::pixels::{self, DesktopFormat};

/// 每次等待新帧的超时与最多尝试次数。
const ACQUIRE_TIMEOUT_MS: u32 = 200;
const ACQUIRE_ATTEMPTS: u32 = 10;

/// Windows 帧源。`DisplayId` 是显示器的 `HMONITOR` 句柄值。
///
/// 每次截图都新建 D3D11 设备和 Duplication，简单但每次需要几十毫秒；
/// 需要更低延迟时（如冻结帧）可以缓存它们。
#[derive(Debug, Default)]
pub struct DxgiFrameSource;

impl DxgiFrameSource {
    pub fn new() -> Self {
        Self
    }
}

impl FrameSource for DxgiFrameSource {
    fn displays(&self) -> Result<Vec<Display>, PlatformError> {
        let mut monitors: Vec<HMONITOR> = Vec::new();
        let ok = unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect_monitor),
                LPARAM(&mut monitors as *mut Vec<HMONITOR> as isize),
            )
        };
        if !ok.as_bool() {
            return Err(PlatformError::new("枚举显示器失败"));
        }
        monitors.into_iter().map(describe_monitor).collect()
    }

    fn cursor_position(&self) -> Result<Point, PlatformError> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.map_err(|e| win_error("读取鼠标位置失败", e))?;
        Ok(Point::new(point.x, point.y))
    }

    fn capture_display(&self, display: DisplayId) -> Result<Image, PlatformError> {
        unsafe { capture_monitor(HMONITOR(display.0 as usize as *mut _)) }
    }
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    true.into()
}

pub(crate) fn describe_monitor(monitor: HMONITOR) -> Result<Display, PlatformError> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Err(PlatformError::new("读取显示器信息失败"));
    }
    let (mut dpi_x, mut dpi_y) = (96, 96);
    // 读取失败时按 100% 缩放处理；缩放比例不影响截图的物理像素。
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let r = info.rcMonitor;
    Ok(Display {
        id: DisplayId(monitor.0 as usize as u64),
        bounds: Rect::new(
            r.left,
            r.top,
            (r.right - r.left) as u32,
            (r.bottom - r.top) as u32,
        ),
        scale_factor: f64::from(dpi_x) / 96.0,
        is_primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

unsafe fn capture_monitor(monitor: HMONITOR) -> Result<Image, PlatformError> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| win_error("创建 DXGI 工厂失败", e))?;
    let (adapter, output, output_desc) = unsafe { find_output(&factory, monitor) }?;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| win_error("创建 D3D11 设备失败", e))?;
    let (Some(device), Some(context)) = (device, context) else {
        return Err(PlatformError::new("创建 D3D11 设备失败：未返回设备"));
    };

    let duplication = unsafe { duplicate_output(&output, &device) }?;
    let rotation = unsafe { duplication.GetDesc() }.Rotation;
    let (desktop, _frame) = unsafe { acquire_frame(&duplication) }?;
    let image = unsafe { copy_to_cpu(&device, &context, &desktop, &output_desc) }?;
    Ok(pixels::rotate(image, rotation))
}

/// 在所有显卡的输出中找到与 `monitor` 对应的那一个。
unsafe fn find_output(
    factory: &IDXGIFactory1,
    monitor: HMONITOR,
) -> Result<(IDXGIAdapter1, IDXGIOutput, DXGI_OUTPUT_DESC), PlatformError> {
    for adapter_index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(win_error("枚举显卡失败", e)),
        };
        for output_index in 0.. {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(win_error("枚举显示输出失败", e)),
            };
            let desc = unsafe { output.GetDesc() }.map_err(|e| win_error("读取显示输出失败", e))?;
            if desc.Monitor == monitor {
                return Ok((adapter, output, desc));
            }
        }
    }
    Err(PlatformError::new("DXGI 中找不到该显示器"))
}

/// 创建 Desktop Duplication。
///
/// 优先用 `IDXGIOutput5::DuplicateOutput1`（Win10 1703+，要求 Per-Monitor V2）声明支持的格式；
/// 注意 HDR / 高级颜色桌面即使只声明 BGRA8 也会给出 FP16 画面，两种格式都要处理。
pub(crate) unsafe fn duplicate_output(
    output: &IDXGIOutput,
    device: &ID3D11Device,
) -> Result<IDXGIOutputDuplication, PlatformError> {
    let formats = [DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT];
    let duplication = match output.cast::<IDXGIOutput5>() {
        Ok(output5) => unsafe { output5.DuplicateOutput1(device, 0, &formats) },
        Err(_) => {
            let output1: IDXGIOutput1 = output
                .cast()
                .map_err(|e| win_error("显示器不支持 Desktop Duplication", e))?;
            unsafe { output1.DuplicateOutput(device) }
        }
    };
    duplication.map_err(|e| win_error("创建 Desktop Duplication 失败", e))
}

/// 已取得的一帧；离开作用域时调用 `ReleaseFrame`。
pub(crate) struct AcquiredFrame<'a>(&'a IDXGIOutputDuplication);

impl Drop for AcquiredFrame<'_> {
    fn drop(&mut self) {
        let _ = unsafe { self.0.ReleaseFrame() };
    }
}

/// 取得当前桌面画面。
///
/// 新建 Duplication 后第一次取到的帧可能还没有桌面画面（`LastPresentTime == 0`，
/// 纹理是全黑的），只有画面更新过的帧才可用，所以跳过这类帧继续等。
pub(crate) unsafe fn acquire_frame(
    duplication: &IDXGIOutputDuplication,
) -> Result<(ID3D11Texture2D, AcquiredFrame<'_>), PlatformError> {
    let mut last_error = None;
    for _ in 0..ACQUIRE_ATTEMPTS {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        match unsafe { duplication.AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut info, &mut resource) }
        {
            Ok(()) => {
                let frame = AcquiredFrame(duplication);
                if info.LastPresentTime == 0 {
                    continue; // 只有鼠标更新、没有桌面画面
                }
                let resource =
                    resource.ok_or_else(|| PlatformError::new("Desktop Duplication 未返回画面"))?;
                let texture = resource
                    .cast::<ID3D11Texture2D>()
                    .map_err(|e| win_error("桌面画面不是 2D 纹理", e))?;
                return Ok((texture, frame));
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => last_error = Some(e),
            Err(e) => return Err(win_error("获取桌面画面失败", e)),
        }
    }
    Err(match last_error {
        Some(e) => win_error("等待桌面画面超时", e),
        None => PlatformError::new("等待桌面画面超时"),
    })
}

/// 把 GPU 上的桌面纹理拷到内存并转换成 RGBA8。
pub(crate) unsafe fn copy_to_cpu(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    output: &DXGI_OUTPUT_DESC,
) -> Result<Image, PlatformError> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let (format, bytes_per_pixel) = match desc.Format {
        DXGI_FORMAT_B8G8R8A8_UNORM => (DesktopFormat::Bgra8, 4),
        DXGI_FORMAT_R16G16B16A16_FLOAT => {
            // 查不到白电平时按 Windows 默认的 SDR 亮度（80 尼特）处理。
            let sdr_white = sdr_white_level(&output.DeviceName).unwrap_or(1.0);
            (DesktopFormat::ScRgbF16 { sdr_white }, 8)
        }
        other => {
            return Err(PlatformError::new(format!(
                "不支持的桌面像素格式 {other:?}"
            )));
        }
    };

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: desc.Width,
        Height: desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: desc.Format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
        .map_err(|e| win_error("创建暂存纹理失败", e))?;
    let staging = staging.ok_or_else(|| PlatformError::new("创建暂存纹理失败"))?;
    unsafe { context.CopyResource(&staging, texture) };

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(|e| win_error("读取暂存纹理失败", e))?;
    let (width, height) = (desc.Width as usize, desc.Height as usize);
    let row_pitch = mapped.RowPitch as usize;
    let len = (row_pitch * height.saturating_sub(1)) + width * bytes_per_pixel;
    let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, len) };
    let rgba = pixels::to_rgba(&format, width, height, row_pitch, data);
    unsafe { context.Unmap(&staging, 0) };
    Ok(Image::from_rgba(desc.Width, desc.Height, rgba))
}
