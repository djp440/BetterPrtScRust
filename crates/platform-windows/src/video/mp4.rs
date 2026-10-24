//! 用 Media Foundation 的 Sink Writer 把纹理编码成 MP4（H.264 High profile，只有视频）。
//!
//! Sink Writer 拿到录屏设备的 DXGI 设备管理器，并允许使用硬件 MFT：BGRA → NV12 的颜色转换和
//! H.264 编码都优先在 GPU 上完成，没有硬件编码器时自动退回系统自带的软件编码器。
//! 输入样本直接引用视频源给出的纹理（[`TextureFrame`]），不拷贝。

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use bps_platform::{PlatformError, VideoFrame, VideoSettings, VideoWriter};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize, IMF2DBuffer, IMFAttributes,
    IMFByteStream, IMFDXGIDeviceManager, IMFMediaType, IMFSinkWriter, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_D3D_MANAGER,
    MF_TRANSCODE_CONTAINERTYPE, MF_VERSION, MFCreateAttributes, MFCreateDXGIDeviceManager,
    MFCreateDXGISurfaceBuffer, MFCreateMediaType, MFCreateSample, MFCreateSinkWriterFromURL,
    MFMediaType_Video, MFSTARTUP_LITE, MFShutdown, MFStartup, MFTranscodeContainerType_MPEG4,
    MFVideoFormat_ARGB32, MFVideoFormat_H264, MFVideoInterlace_Progressive,
    eAVEncH264VProfile_High,
};
use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::{GUID, Interface, w};

use super::{TextureFrame, VideoDevice};
use crate::autostart::ComApartment;
use crate::error::{wide_path, win_error};

/// 每像素每帧的目标码率（比特）：1080p 30 fps 约 7.5 Mbps，录屏的文字和界面足够清晰。
const BITS_PER_PIXEL_PER_FRAME: f64 = 0.12;
/// 宽和高都不小于它时才用硬件编码器。
const MIN_HARDWARE_SIZE: u32 = 256;
/// 码率的上下限（比特每秒）。
const MIN_BITRATE: f64 = 1_000_000.0;
const MAX_BITRATE: f64 = 60_000_000.0;

/// 一段正在编码的 MP4。
///
/// 字段按释放顺序排列：先释放 Sink Writer（关闭文件），再关闭 Media Foundation 和 COM。
pub(super) struct Mp4Writer {
    writer: IMFSinkWriter,
    stream: u32,
    width: u32,
    height: u32,
    /// 每帧的时长（100 纳秒单位）。
    frame_duration: i64,
    _manager: IMFDXGIDeviceManager,
    _device: Rc<VideoDevice>,
    _media_foundation: MediaFoundation,
    _com: ComApartment,
}

impl Mp4Writer {
    pub fn start(
        device: Rc<VideoDevice>,
        path: &Path,
        settings: &VideoSettings,
    ) -> Result<Self, PlatformError> {
        let com = ComApartment::enter()?;
        let media_foundation = MediaFoundation::start()?;
        unsafe {
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)
                .map_err(|e| win_error("创建 DXGI 设备管理器失败", e))?;
            let manager = manager.ok_or_else(|| PlatformError::new("创建 DXGI 设备管理器失败"))?;
            manager
                .ResetDevice(&device.d3d, token)
                .map_err(|e| win_error("设置 DXGI 设备管理器失败", e))?;
            // 硬件编码器对很小的画面不可靠（实测 34 × 34 能建好却在写第一帧时失败），小画面直接用
            // 系统的软件编码器（帧由 Media Foundation 从显存读回，开销很小）；硬件编码器不接受
            // 这种画面时同样退回软件编码器。
            let hardware =
                settings.width >= MIN_HARDWARE_SIZE && settings.height >= MIN_HARDWARE_SIZE;
            let (writer, stream) = if hardware {
                create_sink_writer(path, settings, Some(&manager))
                    .or_else(|_| create_sink_writer(path, settings, None))?
            } else {
                create_sink_writer(path, settings, None)?
            };
            Ok(Self {
                writer,
                stream,
                width: settings.width,
                height: settings.height,
                frame_duration: hundred_nanos(Duration::from_secs(1) / settings.frame_rate),
                _manager: manager,
                _device: device,
                _media_foundation: media_foundation,
                _com: com,
            })
        }
    }
}

impl VideoWriter for Mp4Writer {
    fn write(&mut self, frame: &dyn VideoFrame, timestamp: Duration) -> Result<(), PlatformError> {
        let frame = frame
            .as_any()
            .downcast_ref::<TextureFrame>()
            .ok_or_else(|| PlatformError::new("只能编码录屏视频源给出的帧"))?;
        if (frame.width(), frame.height()) != (self.width, self.height) {
            return Err(PlatformError::new(format!(
                "帧的尺寸 {} × {} 与视频的 {} × {} 不一致",
                frame.width(),
                frame.height(),
                self.width,
                self.height
            )));
        }
        unsafe {
            let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &frame.texture, 0, false)
                .map_err(|e| win_error("包装视频帧失败", e))?;
            let length = buffer
                .cast::<IMF2DBuffer>()
                .and_then(|buffer| buffer.GetContiguousLength())
                .map_err(|e| win_error("读取视频帧大小失败", e))?;
            buffer
                .SetCurrentLength(length)
                .map_err(|e| win_error("设置视频帧大小失败", e))?;
            let sample = MFCreateSample().map_err(|e| win_error("创建视频样本失败", e))?;
            sample
                .AddBuffer(&buffer)
                .and_then(|()| sample.SetSampleTime(hundred_nanos(timestamp)))
                .and_then(|()| sample.SetSampleDuration(self.frame_duration))
                .map_err(|e| win_error("设置视频样本失败", e))?;
            self.writer
                .WriteSample(self.stream, &sample)
                .map_err(|e| win_error("编码视频帧失败", e))
        }
    }

    fn finish(self: Box<Self>) -> Result<(), PlatformError> {
        unsafe { self.writer.Finalize() }.map_err(|e| win_error("写完视频文件失败", e))
    }
}

/// 建好写 `path` 的 Sink Writer（H.264 输出、BGRA 输入）并开始写入，返回它和视频流的序号。
/// 给了 `manager` 时允许硬件 MFT、在 GPU 上转换和编码；否则只用软件编码器。
unsafe fn create_sink_writer(
    path: &Path,
    settings: &VideoSettings,
    manager: Option<&IMFDXGIDeviceManager>,
) -> Result<(IMFSinkWriter, u32), PlatformError> {
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 3).map_err(|e| win_error("创建编码属性失败", e))?;
        let attributes: IMFAttributes =
            attributes.ok_or_else(|| PlatformError::new("创建编码属性失败"))?;
        // 临时文件的扩展名是 .part，容器格式要明确指定。
        attributes
            .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
            .and_then(|()| match manager {
                Some(manager) => attributes
                    .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                    .and_then(|()| attributes.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, manager)),
                None => Ok(()),
            })
            .map_err(|e| win_error("设置编码属性失败", e))?;
        let writer =
            MFCreateSinkWriterFromURL(&wide_path(path), None::<&IMFByteStream>, &attributes)
                .map_err(|e| win_error(&format!("创建视频文件 {} 失败", path.display()), e))?;

        let output = video_type(&MFVideoFormat_H264, settings)?;
        output
            .SetUINT32(&MF_MT_AVG_BITRATE, bitrate(settings))
            .and_then(|()| output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32))
            .map_err(|e| win_error("设置 H.264 参数失败", e))?;
        let stream = writer
            .AddStream(&output)
            .map_err(|e| win_error("添加 H.264 视频流失败", e))?;
        let input = video_type(&MFVideoFormat_ARGB32, settings)?;
        // 正的行跨度 = 自上而下的行序（RGB 默认是自下而上）。
        input
            .SetUINT32(&MF_MT_DEFAULT_STRIDE, settings.width * 4)
            .map_err(|e| win_error("设置输入格式失败", e))?;
        // 不用 B 帧：系统的软件编码器带 B 帧时 MP4 里没有编辑列表，播放器会把整段视频推迟一帧。
        // 每 2 秒一个关键帧，播放器里拖动进度条时定位快。
        let mut parameters = None;
        MFCreateAttributes(&mut parameters, 2).map_err(|e| win_error("创建编码参数失败", e))?;
        let parameters: IMFAttributes =
            parameters.ok_or_else(|| PlatformError::new("创建编码参数失败"))?;
        parameters
            .SetUINT32(&CODECAPI_AVEncMPVDefaultBPictureCount, 0)
            .and_then(|()| parameters.SetUINT32(&CODECAPI_AVEncMPVGOPSize, settings.frame_rate * 2))
            .map_err(|e| win_error("设置编码参数失败", e))?;
        writer
            .SetInputMediaType(stream, &input, &parameters)
            .map_err(|e| {
                win_error(
                    &format!(
                        "没有可用的 H.264 编码器支持 {} × {} 的画面",
                        settings.width, settings.height
                    ),
                    e,
                )
            })?;
        writer
            .BeginWriting()
            .map_err(|e| win_error("开始编码失败", e))?;
        Ok((writer, stream))
    }
}

/// `subtype` 格式、尺寸和帧率与 `settings` 一致的逐行扫描视频类型。
fn video_type(subtype: &GUID, settings: &VideoSettings) -> Result<IMFMediaType, PlatformError> {
    unsafe {
        let media_type = MFCreateMediaType().map_err(|e| win_error("创建媒体类型失败", e))?;
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .and_then(|()| media_type.SetGUID(&MF_MT_SUBTYPE, subtype))
            .and_then(|()| {
                media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            })
            .and_then(|()| {
                media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack(settings.width, settings.height))
            })
            .and_then(|()| media_type.SetUINT64(&MF_MT_FRAME_RATE, pack(settings.frame_rate, 1)))
            .and_then(|()| media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)))
            .map_err(|e| win_error("设置媒体类型失败", e))?;
        Ok(media_type)
    }
}

/// 目标码率（比特每秒）。
fn bitrate(settings: &VideoSettings) -> u32 {
    let pixels = f64::from(settings.width) * f64::from(settings.height);
    (pixels * f64::from(settings.frame_rate) * BITS_PER_PIXEL_PER_FRAME)
        .clamp(MIN_BITRATE, MAX_BITRATE) as u32
}

/// Media Foundation 把两个 32 位数打包成一个 64 位属性（高位在前）。
fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// Media Foundation 的时间单位是 100 纳秒。
fn hundred_nanos(duration: Duration) -> i64 {
    (duration.as_nanos() / 100) as i64
}

/// `MFStartup` / `MFShutdown` 配对（Media Foundation 内部计数，可以嵌套）。
struct MediaFoundation;

impl MediaFoundation {
    fn start() -> Result<Self, PlatformError> {
        // BetterPrtSc.exe 延迟加载 Media Foundation；Windows N 版没有装“媒体功能包”时没有这些 DLL，
        // 延迟加载失败会让进程崩溃，所以先确认它们在。
        for dll in [w!("mfplat.dll"), w!("mfreadwrite.dll")] {
            unsafe { LoadLibraryExW(dll, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.map_err(|_| {
                PlatformError::new(
                    "系统缺少 Media Foundation，无法录制 MP4（Windows N 版需要安装“媒体功能包”）",
                )
            })?;
        }
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }
            .map_err(|e| win_error("启动 Media Foundation 失败", e))?;
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}
