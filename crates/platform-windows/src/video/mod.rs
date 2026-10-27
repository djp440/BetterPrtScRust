//! 录屏：Windows.Graphics.Capture 视频源 + Media Foundation MP4 编码器（spec #1 的录屏后端）。
//!
//! 视频源和编码器共用一个 D3D11 设备（[`VideoDevice`]）：视频源把各显示器的画面拷进这个设备上的
//! 纹理，编码器直接把纹理交给 Media Foundation（颜色转换和 H.264 编码优先走 GPU），画面不经过 CPU。
//! 设备只在录屏期间存在：视频流和编码器都释放后，设备随之释放，常驻进程不长期占用显存。

mod capture;
mod gif;
mod mp4;
mod window;

use std::any::Any;
use std::cell::RefCell;
use std::path::Path;
use std::rc::{Rc, Weak};

use bps_platform::{
    Image, PlatformError, StreamOptions, VideoEncoder, VideoFormat, VideoFrame, VideoSettings,
    VideoSource, VideoStream, VideoTarget, VideoWriter,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{DXGI_OUTPUT_DESC, IDXGIDevice};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::Interface;

use crate::error::win_error;
use crate::frame_source::copy_to_cpu;

/// 录屏的视频源：Windows.Graphics.Capture 捕获目标区域所在的每块显示器，按区域裁剪拼接成
/// GPU 纹理。需要 Windows 10 1903 或更高版本；鼠标指针开关需要 Windows 10 2004 或更高版本。
#[derive(Debug, Default)]
pub struct WgcVideoSource;

impl WgcVideoSource {
    pub fn new() -> Self {
        Self
    }
}

impl VideoSource for WgcVideoSource {
    fn open(
        &mut self,
        target: VideoTarget,
        options: StreamOptions,
    ) -> Result<Box<dyn VideoStream>, PlatformError> {
        let device = VideoDevice::shared()?;
        match target {
            VideoTarget::Region(region) => Ok(Box::new(capture::RegionStream::open(
                device, region, options,
            )?)),
            VideoTarget::Window {
                window,
                width,
                height,
            } => Ok(Box::new(window::WindowStream::open(
                device, window, width, height, options,
            )?)),
        }
    }
}

/// 录屏的编码器：Media Foundation Sink Writer，MP4 里是 H.264 视频（优先硬件编码）；GIF 由纯 Rust 编码器在 CPU 上编码。
#[derive(Debug, Default)]
pub struct MfVideoEncoder;

impl MfVideoEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl VideoEncoder for MfVideoEncoder {
    fn start(
        &mut self,
        path: &Path,
        settings: &VideoSettings,
    ) -> Result<Box<dyn VideoWriter>, PlatformError> {
        match settings.format {
            VideoFormat::Mp4 => Ok(Box::new(mp4::Mp4Writer::start(
                VideoDevice::shared()?,
                path,
                settings,
            )?)),
            VideoFormat::Gif => Ok(Box::new(gif::GifWriter::start(path, settings)?)),
        }
    }
}

/// 录屏用的 D3D11 设备（同时包成 WinRT 的 `IDirect3DDevice` 给 Windows.Graphics.Capture 用）。
pub(crate) struct VideoDevice {
    pub d3d: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub winrt: IDirect3DDevice,
}

thread_local! {
    /// 当前线程上正在使用的录屏设备；没有人持有时自动释放。
    static SHARED: RefCell<Weak<VideoDevice>> = const { RefCell::new(Weak::new()) };
}

impl VideoDevice {
    /// 正在使用的录屏设备；没有时新建一个。
    fn shared() -> Result<Rc<VideoDevice>, PlatformError> {
        if let Some(device) = SHARED.with(|shared| shared.borrow().upgrade()) {
            return Ok(device);
        }
        let device = Rc::new(VideoDevice::create()?);
        SHARED.with(|shared| *shared.borrow_mut() = Rc::downgrade(&device));
        Ok(device)
    }

    fn create() -> Result<Self, PlatformError> {
        // 视频支持让 Media Foundation 能在这个设备上做颜色转换和硬件编码；
        // 个别驱动不支持时退回普通设备（编码器会改用软件路径）。
        let (d3d, context) =
            create_device(D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT)
                .or_else(|_| create_device(D3D11_CREATE_DEVICE_BGRA_SUPPORT))?;
        unsafe {
            // Media Foundation 会在自己的线程上使用这个设备。
            if let Ok(multithread) = d3d.cast::<ID3D11Multithread>() {
                let _ = multithread.SetMultithreadProtected(true);
            }
            let dxgi: IDXGIDevice = d3d.cast().map_err(|e| win_error("读取 DXGI 设备失败", e))?;
            let winrt = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .and_then(|inspectable| inspectable.cast::<IDirect3DDevice>())
                .map_err(|e| win_error("创建 WinRT Direct3D 设备失败", e))?;
            Ok(Self {
                d3d,
                context,
                winrt,
            })
        }
    }
}

fn create_device(
    flags: D3D11_CREATE_DEVICE_FLAG,
) -> Result<(ID3D11Device, ID3D11DeviceContext), PlatformError> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            Some(&[
                D3D_FEATURE_LEVEL_11_1,
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| win_error("创建录屏用的 D3D11 设备失败", e))?;
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(PlatformError::new(
            "创建录屏用的 D3D11 设备失败：未返回设备",
        )),
    }
}

/// 视频源给出的一帧：录屏设备上的一张 BGRA8 纹理。纹理写好后不再修改，编码器可以放心地
/// 异步读取它。
pub(crate) struct TextureFrame {
    device: Rc<VideoDevice>,
    pub texture: ID3D11Texture2D,
    width: u32,
    height: u32,
}

impl VideoFrame for TextureFrame {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn to_image(&self) -> Result<Image, PlatformError> {
        // BGRA8 纹理用不到输出描述（只有 HDR 的 FP16 才用其中的显示器名查白电平）。
        unsafe {
            copy_to_cpu(
                &self.device.d3d,
                &self.device.context,
                &self.texture,
                &DXGI_OUTPUT_DESC::default(),
            )
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
