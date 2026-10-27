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
use std::time::Duration;

use bps_platform::{
    Image, PlatformError, StreamOptions, VideoCodec, VideoEncoder, VideoFormat, VideoFrame,
    VideoSettings, VideoSource, VideoStream, VideoTarget, VideoWriter,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice,
    ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
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
            VideoFormat::Mp4 => {
                let device = VideoDevice::shared()?;
                check_codec(&device, settings.codec)?;
                Ok(Box::new(mp4::Mp4Writer::start(device, path, settings)?))
            }
            VideoFormat::Gif => Ok(Box::new(gif::GifWriter::start(path, settings)?)),
        }
    }
}

/// 自检用的画面边长：不小于硬件编码器的下限，和录屏时一样走硬件编码。
const CHECK_SIZE: u32 = 256;
/// 自检编码的帧数。
const CHECK_FRAMES: u32 = 10;

thread_local! {
    /// H.265 / AV1 的自检结果（每个线程只试一次）。
    static CHECKED: RefCell<Vec<(VideoCodec, Result<(), PlatformError>)>> =
        const { RefCell::new(Vec::new()) };
}

/// 返回不能用的视频编码和原因（H.265、AV1 逐个自检，见 [`check_codec`]）。
/// 设置界面用它把不能用的编码置灰；第一次调用约需一秒。
pub fn unsupported_video_codecs() -> Vec<(VideoCodec, String)> {
    let candidates = [VideoCodec::H265, VideoCodec::Av1];
    let device = match VideoDevice::shared() {
        Ok(device) => device,
        Err(e) => return candidates.map(|codec| (codec, e.to_string())).to_vec(),
    };
    candidates
        .into_iter()
        .filter_map(|codec| {
            check_codec(&device, codec)
                .err()
                .map(|e| (codec, e.to_string()))
        })
        .collect()
}

/// 确认 `codec` 能从头到尾编出一个 MP4：用它把几帧空白画面编进临时文件并写完。
/// 只建编码器不够：实测有的 AV1 硬件编码器能建好、能收帧，却在写完文件时才失败（缺少序列头），
/// 那样整段录像就丢了。H.264 系统总有软件编码器，不必试；结果按线程缓存。
fn check_codec(device: &Rc<VideoDevice>, codec: VideoCodec) -> Result<(), PlatformError> {
    if codec == VideoCodec::H264 {
        return Ok(());
    }
    if let Some(result) = CHECKED.with(|checked| {
        checked
            .borrow()
            .iter()
            .find(|(c, _)| *c == codec)
            .map(|(_, result)| result.clone())
    }) {
        return result;
    }
    let path = std::env::temp_dir().join(format!(
        "bps-codec-check-{}-{codec:?}.mp4",
        std::process::id()
    ));
    let result = encode_blank(device, &path, codec);
    let _ = std::fs::remove_file(&path);
    CHECKED.with(|checked| checked.borrow_mut().push((codec, result.clone())));
    result
}

fn encode_blank(
    device: &Rc<VideoDevice>,
    path: &Path,
    codec: VideoCodec,
) -> Result<(), PlatformError> {
    let settings = VideoSettings {
        format: VideoFormat::Mp4,
        width: CHECK_SIZE,
        height: CHECK_SIZE,
        frame_rate: 30,
        codec,
        bitrate: 1_000_000,
        audio: None,
    };
    let mut writer = Box::new(mp4::Mp4Writer::start(device.clone(), path, &settings)?);
    let frame = blank_frame(device, CHECK_SIZE, CHECK_SIZE)?;
    for n in 0..CHECK_FRAMES {
        writer.write(&frame, Duration::from_secs(1) * n / settings.frame_rate)?;
    }
    writer.finish()
}

/// 录屏设备上一张全黑的 BGRA8 纹理。
fn blank_frame(
    device: &Rc<VideoDevice>,
    width: u32,
    height: u32,
) -> Result<TextureFrame, PlatformError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let pixels = vec![0u8; (width * height * 4) as usize];
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: width * 4,
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    unsafe {
        device
            .d3d
            .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
    }
    .map_err(|e| win_error("创建自检画面失败", e))?;
    Ok(TextureFrame {
        device: device.clone(),
        texture: texture.ok_or_else(|| PlatformError::new("创建自检画面失败"))?,
        width,
        height,
    })
}

/// 系统里没有解码器的视频编码：用它录的视频系统播放器放不了（要装扩展或换播放器）。
/// 列不出解码器时当作都有，不误报。
pub fn undecodable_video_codecs() -> Vec<VideoCodec> {
    VideoCodec::ALL
        .into_iter()
        .filter(|&codec| matches!(mp4::has_decoder(codec), Ok(false)))
        .collect()
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
