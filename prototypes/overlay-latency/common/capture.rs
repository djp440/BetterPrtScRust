//! 一次性原型代码（ticket #2）：用 DXGI Desktop Duplication 拍冻结帧。
//! 通过 `#[path = "../../common/capture.rs"]` 引入，不是共享 crate。
//!
//! 每块显示器（DXGI output）单独复制；output 只能用“它所属适配器”上的 D3D11 设备去复制，
//! 所以按适配器各建一个设备（混合显卡笔记本上外接屏和内屏可能挂在不同适配器上）。
//! 进程必须先声明 Per-Monitor V2，这样 DesktopCoordinates 与纹理尺寸都是物理像素。

#![allow(dead_code)]

use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::{Interface, Result};

#[derive(Clone, Debug)]
pub struct OutputInfo {
    pub name: String,
    /// 在虚拟桌面坐标系中的物理像素矩形。
    pub rect: RECT,
    pub rotation: DXGI_MODE_ROTATION,
    pub monitor: HMONITOR,
    /// 有效 DPI（96 = 100%）。
    pub dpi: u32,
    pub primary: bool,
    pub adapter_index: usize,
}

impl OutputInfo {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
}

pub struct AdapterCtx {
    pub adapter: IDXGIAdapter1,
    pub desc: DXGI_ADAPTER_DESC1,
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub outputs: Vec<(IDXGIOutput1, OutputInfo)>,
}

pub struct Capturer {
    factory: IDXGIFactory1,
    pub adapters: Vec<AdapterCtx>,
}

/// GPU 上的冻结帧：纹理位于 `adapter_index` 对应的 D3D11 设备上。
pub struct GpuFrame {
    pub info: OutputInfo,
    pub texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
}

/// CPU 上的冻结帧：紧密排列的 BGRA8。
pub struct CpuFrame {
    pub info: OutputInfo,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub fn adapter_name(desc: &DXGI_ADAPTER_DESC1) -> String {
    let n = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
    String::from_utf16_lossy(&desc.Description[..n])
}

/// 只枚举显示器（不建 D3D 设备），用于日志：`名称@x,y,宽x高,dpiN,适配器;...`。
pub fn describe_outputs() -> Result<String> {
    let mut parts = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut ai = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(ai) {
            ai += 1;
            let desc = adapter.GetDesc1()?;
            let mut oi = 0u32;
            while let Ok(output) = adapter.EnumOutputs(oi) {
                oi += 1;
                let od = output.GetDesc()?;
                if !od.AttachedToDesktop.as_bool() {
                    continue;
                }
                let n = od.DeviceName.iter().position(|&c| c == 0).unwrap_or(32);
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(od.Monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let r = od.DesktopCoordinates;
                parts.push(format!(
                    "{}@{},{},{}x{},dpi{},rot{},{}",
                    String::from_utf16_lossy(&od.DeviceName[..n]),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    dx,
                    od.Rotation.0,
                    adapter_name(&desc).replace(' ', "_")
                ));
            }
        }
    }
    Ok(parts.join(";"))
}

pub fn create_device(adapter: &IDXGIAdapter1) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((device.unwrap(), context.unwrap()))
}

impl Capturer {
    /// 枚举所有挂着桌面显示器的适配器，并为每个适配器创建 D3D11 设备。
    pub fn new() -> Result<Self> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut adapters = Vec::new();
            let mut ai = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(ai) {
                ai += 1;
                let desc = adapter.GetDesc1()?;
                let mut outputs = Vec::new();
                let mut oi = 0u32;
                while let Ok(output) = adapter.EnumOutputs(oi) {
                    oi += 1;
                    let od = output.GetDesc()?;
                    if !od.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    let n = od.DeviceName.iter().position(|&c| c == 0).unwrap_or(32);
                    let (mut dx, mut dy) = (96u32, 96u32);
                    let _ = GetDpiForMonitor(od.Monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                    let rect = od.DesktopCoordinates;
                    outputs.push((
                        output.cast::<IDXGIOutput1>()?,
                        OutputInfo {
                            name: String::from_utf16_lossy(&od.DeviceName[..n]),
                            rect,
                            rotation: od.Rotation,
                            monitor: od.Monitor,
                            dpi: dx,
                            primary: rect.left == 0 && rect.top == 0,
                            adapter_index: 0,
                        },
                    ));
                }
                if outputs.is_empty() {
                    continue;
                }
                let index = adapters.len();
                for (_, info) in outputs.iter_mut() {
                    info.adapter_index = index;
                }
                let (device, context) = create_device(&adapter)?;
                adapters.push(AdapterCtx {
                    adapter,
                    desc,
                    device,
                    context,
                    outputs,
                });
            }
            Ok(Self { factory, adapters })
        }
    }

    /// 显示器拓扑变化（插拔、改分辨率、改缩放）后 factory 会失效，需要重建 Capturer。
    pub fn is_current(&self) -> bool {
        unsafe { self.factory.IsCurrent().as_bool() }
    }

    pub fn outputs(&self) -> impl Iterator<Item = &OutputInfo> {
        self.adapters
            .iter()
            .flat_map(|a| a.outputs.iter().map(|(_, i)| i))
    }

    pub fn primary(&self) -> Option<&OutputInfo> {
        self.outputs().find(|o| o.primary)
    }

    /// 所有显示器的外接矩形（物理像素，可能有负坐标）。
    pub fn virtual_rect(&self) -> RECT {
        let mut r = RECT {
            left: i32::MAX,
            top: i32::MAX,
            right: i32::MIN,
            bottom: i32::MIN,
        };
        for o in self.outputs() {
            r.left = r.left.min(o.rect.left);
            r.top = r.top.min(o.rect.top);
            r.right = r.right.max(o.rect.right);
            r.bottom = r.bottom.max(o.rect.bottom);
        }
        r
    }

    /// 复制一块显示器的当前画面，`f` 在 ReleaseFrame 之前拿到桌面纹理。
    /// 返回值附带尝试次数，用于观察“首帧只有鼠标更新”的情况。
    fn grab<T>(
        ctx: &AdapterCtx,
        output: &IDXGIOutput1,
        mut f: impl FnMut(&ID3D11Texture2D, &D3D11_TEXTURE2D_DESC) -> Result<T>,
    ) -> Result<(T, u32)> {
        unsafe {
            // 优先 DuplicateOutput1（可指定格式，HDR 显示器下也拿到 BGRA8）；本机实测 Per-Monitor V2
            // 进程用旧的 DuplicateOutput 同样可用。设 BPS_DUP_LEGACY 可强制走旧接口对比，
            // BPS_DUP_DEBUG 打印每次 Acquire 的帧信息到 stderr。
            let debug = std::env::var("BPS_DUP_DEBUG").is_ok();
            let t_dup = std::time::Instant::now();
            let dup = if std::env::var("BPS_DUP_LEGACY").is_ok() {
                output.DuplicateOutput(&ctx.device)
            } else {
                match output.cast::<IDXGIOutput5>() {
                    Ok(o5) => o5.DuplicateOutput1(&ctx.device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
                    Err(_) => output.DuplicateOutput(&ctx.device),
                }
            };
            if debug {
                eprintln!(
                    "dup create {:?} in {:.2}ms",
                    dup.as_ref().map(|_| ()),
                    t_dup.elapsed().as_secs_f64() * 1000.0
                );
            }
            let dup = dup?;
            let mut attempts = 0;
            loop {
                attempts += 1;
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut res: Option<IDXGIResource> = None;
                let r = dup.AcquireNextFrame(500, &mut info, &mut res);
                if debug {
                    eprintln!(
                        "  acquire#{attempts} {:?} t={:.2}ms LastPresentTime={} Accumulated={} LastMouseUpdate={} meta={} ptrVisible={}",
                        r.as_ref().map(|_| ()),
                        t_dup.elapsed().as_secs_f64() * 1000.0,
                        info.LastPresentTime,
                        info.AccumulatedFrames,
                        info.LastMouseUpdateTime,
                        info.TotalMetadataBufferSize,
                        info.PointerPosition.Visible.as_bool()
                    );
                }
                match r {
                    Ok(()) => {}
                    Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT && attempts < 4 => continue,
                    Err(e) => return Err(e),
                }
                if info.LastPresentTime == 0 && attempts < 4 {
                    // 实测：新建复制后第一次 Acquire 总是“只有鼠标更新”（LastPresentTime=0），
                    // 桌面图像要等下一次 DWM 合成（本机 2–7 ms）才到，继续等。
                    dup.ReleaseFrame()?;
                    continue;
                }
                let tex: ID3D11Texture2D = res.unwrap().cast()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                tex.GetDesc(&mut desc);
                if debug {
                    // HDR 桌面下即使只声明 BGRA8 也可能拿到 FP16（见 #3 笔记），原型假设 BGRA8。
                    eprintln!(
                        "  frame format={:?} {}x{}",
                        desc.Format, desc.Width, desc.Height
                    );
                }
                let r = f(&tex, &desc);
                dup.ReleaseFrame()?;
                return Ok((r?, attempts));
            }
        }
    }

    /// 把每块显示器复制到各自适配器上的一张 GPU 纹理（可作为 D2D 位图 / SRV）。
    pub fn capture_gpu(&self) -> Result<(Vec<GpuFrame>, u32)> {
        let mut frames = Vec::new();
        let mut max_attempts = 0;
        for ctx in &self.adapters {
            for (output, info) in &ctx.outputs {
                let (frame, attempts) = Self::grab(ctx, output, |src, desc| unsafe {
                    let mut d = *desc;
                    d.Usage = D3D11_USAGE_DEFAULT;
                    d.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                    d.CPUAccessFlags = 0;
                    d.MiscFlags = 0;
                    d.MipLevels = 1;
                    d.ArraySize = 1;
                    let mut dst = None;
                    ctx.device.CreateTexture2D(&d, None, Some(&mut dst))?;
                    let dst = dst.unwrap();
                    ctx.context.CopyResource(&dst, src);
                    Ok(GpuFrame {
                        info: info.clone(),
                        texture: dst,
                        width: d.Width,
                        height: d.Height,
                    })
                })?;
                max_attempts = max_attempts.max(attempts);
                frames.push(frame);
            }
        }
        Ok((frames, max_attempts))
    }

    /// 把某个适配器上的 GPU 纹理读回 CPU（BGRA8，紧密排列）。用于跨适配器的显示器。
    pub fn read_back(&self, frame: &GpuFrame) -> Result<Vec<u8>> {
        let ctx = &self.adapters[frame.info.adapter_index];
        unsafe {
            let mut d = D3D11_TEXTURE2D_DESC::default();
            frame.texture.GetDesc(&mut d);
            d.Usage = D3D11_USAGE_STAGING;
            d.BindFlags = 0;
            d.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            ctx.device.CreateTexture2D(&d, None, Some(&mut staging))?;
            let staging = staging.unwrap();
            ctx.context.CopyResource(&staging, &frame.texture);
            let (w, h) = (frame.width, frame.height);
            let mut data = vec![0u8; (w * h * 4) as usize];
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let row = (w * 4) as usize;
            for y in 0..h as usize {
                let src = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                std::ptr::copy_nonoverlapping(src, data.as_mut_ptr().add(y * row), row);
            }
            ctx.context.Unmap(&staging, 0);
            Ok(data)
        }
    }

    /// 把每块显示器读回 CPU（BGRA8，紧密排列）。
    pub fn capture_cpu(&self) -> Result<(Vec<CpuFrame>, u32)> {
        let mut frames = Vec::new();
        let mut max_attempts = 0;
        for ctx in &self.adapters {
            for (output, info) in &ctx.outputs {
                let (frame, attempts) = Self::grab(ctx, output, |src, desc| unsafe {
                    let mut d = *desc;
                    d.Usage = D3D11_USAGE_STAGING;
                    d.BindFlags = 0;
                    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                    d.MiscFlags = 0;
                    d.MipLevels = 1;
                    d.ArraySize = 1;
                    let mut staging = None;
                    ctx.device.CreateTexture2D(&d, None, Some(&mut staging))?;
                    let staging = staging.unwrap();
                    ctx.context.CopyResource(&staging, src);
                    Ok((staging, d.Width, d.Height))
                })?;
                // ReleaseFrame 之后再 Map，尽量缩短占用桌面帧的时间。
                let (staging, w, h) = frame;
                let mut data = vec![0u8; (w * h * 4) as usize];
                unsafe {
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    ctx.context
                        .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                    let row = (w * 4) as usize;
                    for y in 0..h as usize {
                        let src = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                        std::ptr::copy_nonoverlapping(src, data.as_mut_ptr().add(y * row), row);
                    }
                    ctx.context.Unmap(&staging, 0);
                }
                max_attempts = max_attempts.max(attempts);
                frames.push(CpuFrame {
                    info: info.clone(),
                    width: w,
                    height: h,
                    data,
                });
            }
        }
        Ok((frames, max_attempts))
    }
}
