//! 用 Media Foundation 的 Sink Writer 把纹理编码成 MP4（H.264 High profile、H.265 Main profile 或
//! AV1），可以带一条 AAC 音轨（系统自带的 AAC 编码器：输入 16 位 PCM，44.1 / 48 kHz、单声道或双声道）。
//! 码率由 `core` 算好放在 [`VideoSettings::bitrate`] 里。
//!
//! Sink Writer 拿到录屏设备的 DXGI 设备管理器，并允许使用硬件 MFT：BGRA → NV12 的颜色转换和
//! 视频编码都优先在 GPU 上完成，没有硬件编码器时自动退回系统的软件编码器（H.264 系统自带；
//! H.265 要装“HEVC 视频扩展”；AV1 通常只有显卡的硬件编码器）。
//! 输入样本直接引用视频源给出的纹理（[`TextureFrame`]），不拷贝。

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use bps_platform::{
    AudioFormat, PlatformError, VideoCodec, VideoFrame, VideoSettings, VideoWriter,
};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize, IMF2DBuffer, IMFActivate,
    IMFAttributes, IMFByteStream, IMFDXGIDeviceManager, IMFMediaSink, IMFMediaType, IMFSinkWriter,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_MPEG2_PROFILE,
    MF_MT_MPEG4_SAMPLE_DESCRIPTION, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_D3D_MANAGER, MF_SINK_WRITER_MEDIASINK,
    MF_TRANSCODE_CONTAINERTYPE, MF_VERSION, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Audio,
    MFMediaType_Video, MFSTARTUP_LITE, MFShutdown, MFStartup, MFT_CATEGORY_VIDEO_DECODER,
    MFT_ENUM_FLAG_ALL, MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFTranscodeContainerType_MPEG4,
    MFVideoFormat_ARGB32, MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC,
    MFVideoInterlace_Progressive, eAVEncH264VProfile_High, eAVEncH265VProfile_Main_420_8,
};
use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::{GUID, Interface, w};

use super::{TextureFrame, VideoDevice};
use crate::autostart::ComApartment;
use crate::error::{wide_path, win_error};

/// 宽和高都不小于它时才用硬件编码器。
const MIN_HARDWARE_SIZE: u32 = 256;
/// AAC 音轨的码率（字节每秒）：24000 即 192 kbps，编码器支持的最高档。
const AAC_BYTES_PER_SECOND: u32 = 24_000;
/// 一个 AAC 帧的采样帧数：音轨一个样本也没有时写入这么多静音，免得写完文件时出错。
const AAC_FRAME: usize = 1024;

/// 一段正在编码的 MP4。
///
/// 字段按释放顺序排列：先释放 Sink Writer（关闭文件），再关闭 Media Foundation 和 COM。
pub(super) struct Mp4Writer {
    writer: IMFSinkWriter,
    stream: u32,
    width: u32,
    height: u32,
    codec: VideoCodec,
    /// 每帧的时长（100 纳秒单位）。
    frame_duration: i64,
    /// 音轨；没有声音时没有。
    audio: Option<AudioStreamState>,
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
            let (writer, stream, audio) = if hardware {
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
                codec: settings.codec,
                frame_duration: hundred_nanos(Duration::from_secs(1) / settings.frame_rate),
                audio: audio
                    .zip(settings.audio)
                    .map(|(stream, format)| AudioStreamState {
                        stream,
                        format,
                        written: 0,
                    }),
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

    fn write_audio(&mut self, samples: &[f32]) -> Result<(), PlatformError> {
        let audio = self
            .audio
            .as_mut()
            .ok_or_else(|| PlatformError::new("这段视频没有音轨"))?;
        audio.write(&self.writer, samples)
    }

    fn finish(mut self: Box<Self>) -> Result<(), PlatformError> {
        if let Some(audio) = self.audio.as_mut().filter(|audio| audio.written == 0) {
            let silence = vec![0.0; AAC_FRAME * usize::from(audio.format.channels)];
            audio.write(&self.writer, &silence)?;
        }
        if self.codec == VideoCodec::Av1 {
            set_av1_sample_description(&self.writer, self.stream, self.width, self.height)?;
        }
        unsafe { self.writer.Finalize() }.map_err(|e| win_error("写完视频文件失败", e))
    }
}

/// Windows 10 的 MP4 sink 已拿到 AV1 OBU 序列头，却不会据此生成 av01 / av1C。
/// 按 MPEG-4 File Sink 的自定义格式约定，在 Finalize 前补完整 stsd；仍由系统封装音视频。
/// https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink#sample-description-box
fn set_av1_sample_description(
    writer: &IMFSinkWriter,
    stream: u32,
    width: u32,
    height: u32,
) -> Result<(), PlatformError> {
    unsafe {
        let mut raw = std::ptr::null_mut();
        writer
            .GetServiceForStream(
                MF_SINK_WRITER_MEDIASINK.0,
                &GUID::zeroed(),
                &IMFMediaSink::IID,
                &mut raw,
            )
            .map_err(|e| win_error("读取 MP4 封装器失败", e))?;
        let sink = IMFMediaSink::from_raw(raw);
        let handler = sink
            .GetStreamSinkByIndex(stream)
            .and_then(|stream| stream.GetMediaTypeHandler())
            .map_err(|e| win_error("读取 AV1 视频流失败", e))?;
        let media_type = handler
            .GetCurrentMediaType()
            .map_err(|e| win_error("读取 AV1 输出格式失败", e))?;
        let size = media_type
            .GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER)
            .map_err(|e| win_error("读取 AV1 序列头失败", e))?;
        let mut header = vec![0; size as usize];
        media_type
            .GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut header, None)
            .map_err(|e| win_error("读取 AV1 序列头失败", e))?;
        let description = super::av1::sample_description(&header, width, height)?;
        media_type
            .SetBlob(&MF_MT_MPEG4_SAMPLE_DESCRIPTION, &description)
            .and_then(|()| handler.SetCurrentMediaType(&media_type))
            .map_err(|e| win_error("设置 AV1 MP4 格式描述失败", e))
    }
}

/// MP4 里的音轨。
struct AudioStreamState {
    stream: u32,
    format: AudioFormat,
    /// 已经写入的采样帧数。
    written: u64,
}

impl AudioStreamState {
    /// 把交错排列的浮点样本转成 16 位 PCM，接在已写入的声音后面。
    fn write(&mut self, writer: &IMFSinkWriter, samples: &[f32]) -> Result<(), PlatformError> {
        let channels = usize::from(self.format.channels);
        let frames = (samples.len() / channels) as u64;
        if frames == 0 {
            return Ok(());
        }
        let bytes: Vec<u8> = samples[..frames as usize * channels]
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes())
            .collect();
        let rate = u64::from(self.format.sample_rate);
        let time = |frames: u64| (u128::from(frames) * 10_000_000 / u128::from(rate)) as i64;
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes.len() as u32)
                .map_err(|e| win_error("创建音频缓冲失败", e))?;
            let mut data = std::ptr::null_mut();
            buffer
                .Lock(&mut data, None, None)
                .map_err(|e| win_error("写入音频缓冲失败", e))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            buffer
                .Unlock()
                .and_then(|()| buffer.SetCurrentLength(bytes.len() as u32))
                .map_err(|e| win_error("写入音频缓冲失败", e))?;
            let sample = MFCreateSample().map_err(|e| win_error("创建音频样本失败", e))?;
            let start = time(self.written);
            sample
                .AddBuffer(&buffer)
                .and_then(|()| sample.SetSampleTime(start))
                .and_then(|()| sample.SetSampleDuration(time(self.written + frames) - start))
                .map_err(|e| win_error("设置音频样本失败", e))?;
            writer
                .WriteSample(self.stream, &sample)
                .map_err(|e| win_error("编码声音失败", e))?;
        }
        self.written += frames;
        Ok(())
    }
}

/// `codec` 的 Media Foundation 输出格式。
fn subtype(codec: VideoCodec) -> GUID {
    match codec {
        VideoCodec::H264 => MFVideoFormat_H264,
        VideoCodec::H265 => MFVideoFormat_HEVC,
        VideoCodec::Av1 => MFVideoFormat_AV1,
    }
}

/// 系统里有没有 `codec` 的解码器（系统播放器靠它播放录好的视频）。
/// Windows 10 的 H.265 解码器要从 Microsoft Store 装“HEVC 视频扩展”，AV1 要装“AV1 视频扩展”。
pub(super) fn has_decoder(codec: VideoCodec) -> Result<bool, PlatformError> {
    let _com = ComApartment::enter()?;
    let _media_foundation = MediaFoundation::start()?;
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype(codec),
    };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0;
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_ALL,
            Some(&input),
            None,
            &mut activates,
            &mut count,
        )
        .map_err(|e| win_error("无法列出视频解码器", e))?;
        if !activates.is_null() {
            // 释放每个 IMFActivate，再释放数组本身。
            for activate in std::slice::from_raw_parts_mut(activates, count as usize) {
                drop(activate.take());
            }
            windows::Win32::System::Com::CoTaskMemFree(Some(activates.cast()));
        }
    }
    Ok(count > 0)
}

/// 建好写 `path` 的 Sink Writer（按 `settings.codec` 编码、BGRA 输入；有声音时再加一条 AAC 音轨）并开始写入，
/// 返回它、视频流的序号和音频流的序号。
/// 给了 `manager` 时允许硬件 MFT、在 GPU 上转换和编码；否则只用软件编码器。
unsafe fn create_sink_writer(
    path: &Path,
    settings: &VideoSettings,
    manager: Option<&IMFDXGIDeviceManager>,
) -> Result<(IMFSinkWriter, u32, Option<u32>), PlatformError> {
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

        let codec = settings.codec.label();
        let profile = match settings.codec {
            VideoCodec::H264 => Some(eAVEncH264VProfile_High.0 as u32),
            VideoCodec::H265 => Some(eAVEncH265VProfile_Main_420_8.0 as u32),
            VideoCodec::Av1 => None,
        };
        let output = video_type(&subtype(settings.codec), settings)?;
        output
            .SetUINT32(&MF_MT_AVG_BITRATE, settings.bitrate)
            .and_then(|()| match profile {
                Some(profile) => output.SetUINT32(&MF_MT_MPEG2_PROFILE, profile),
                None => Ok(()),
            })
            .map_err(|e| win_error(&format!("设置 {codec} 参数失败"), e))?;
        let stream = writer
            .AddStream(&output)
            .map_err(|e| win_error(&format!("添加 {codec} 视频流失败"), e))?;
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
                        "没有可用的 {codec} 编码器支持 {} × {} 的画面",
                        settings.width, settings.height
                    ),
                    e,
                )
            })?;
        let audio = match settings.audio {
            Some(format) => Some(add_audio_stream(&writer, format)?),
            None => None,
        };
        writer
            .BeginWriting()
            .map_err(|e| win_error("开始编码失败", e))?;
        Ok((writer, stream, audio))
    }
}

/// 给 Sink Writer 加一条 AAC 音轨（输入 16 位 PCM），返回它的序号。
unsafe fn add_audio_stream(
    writer: &IMFSinkWriter,
    format: AudioFormat,
) -> Result<u32, PlatformError> {
    unsafe {
        let output = audio_type(&MFAudioFormat_AAC, format)?;
        output
            .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AAC_BYTES_PER_SECOND)
            .map_err(|e| win_error("设置 AAC 参数失败", e))?;
        let stream = writer
            .AddStream(&output)
            .map_err(|e| win_error("添加 AAC 音轨失败", e))?;
        let input = audio_type(&MFAudioFormat_PCM, format)?;
        let block = u32::from(format.channels) * 2;
        input
            .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block)
            .and_then(|()| {
                input.SetUINT32(
                    &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                    format.sample_rate * block,
                )
            })
            .map_err(|e| win_error("设置音频输入格式失败", e))?;
        writer
            .SetInputMediaType(stream, &input, None)
            .map_err(|e| {
                win_error(
                    &format!(
                        "没有可用的 AAC 编码器支持 {} Hz、{} 声道的声音",
                        format.sample_rate, format.channels
                    ),
                    e,
                )
            })?;
        Ok(stream)
    }
}

/// `subtype` 格式、16 位、采样率和声道数与 `format` 一致的音频类型。
fn audio_type(subtype: &GUID, format: AudioFormat) -> Result<IMFMediaType, PlatformError> {
    unsafe {
        let media_type = MFCreateMediaType().map_err(|e| win_error("创建媒体类型失败", e))?;
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .and_then(|()| media_type.SetGUID(&MF_MT_SUBTYPE, subtype))
            .and_then(|()| media_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16))
            .and_then(|()| {
                media_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, format.sample_rate)
            })
            .and_then(|()| {
                media_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(format.channels))
            })
            .map_err(|e| win_error("设置媒体类型失败", e))?;
        Ok(media_type)
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
