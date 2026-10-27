//! 录屏的系统声音：WASAPI 共享模式下回环采集默认输出设备（“正在播放的声音”）。
//!
//! 统一请求 48 kHz、双声道、32 位浮点：设备的混音格式不同时由 WASAPI 转换
//! （`AUTOCONVERTPCM`），MP4 的 AAC 编码器正好接受这种采样率和声道数。
//! 不开采集线程：录屏的节拍每次都来读取，缓冲留 2 秒，足够吸收事件循环偶尔的卡顿。
//! 回环采集在系统静音期间不产生数据，空白由 `core` 按采集时间补静音。
//!
//! 采集时间按“读取时缓冲里的声音刚好采集到此刻为止”往回推算（误差不超过一个包的周期，约 10 毫秒）。
//! 不用 `GetBuffer` 报告的 QPC 时间：回环流上它不可靠（实测刚开始的几个包时间相同，之后的包与
//! 样本数对不上）。

use std::time::Duration;

use bps_platform::{AudioChunk, AudioFormat, AudioSource, AudioStream, Clock, PlatformError};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient,
    IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

use crate::autostart::ComApartment;
use crate::clock::WindowsClock;
use crate::error::win_error;

/// 采集到的声音的格式。
const FORMAT: AudioFormat = AudioFormat {
    sample_rate: 48_000,
    channels: 2,
};
/// `WAVE_FORMAT_IEEE_FLOAT`。
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
/// WASAPI 缓冲的时长（100 纳秒单位）：2 秒。
const BUFFER_DURATION: i64 = 20_000_000;

/// 系统声音的音频源：默认输出设备的 WASAPI 回环。
#[derive(Debug, Default)]
pub struct WasapiAudioSource;

impl WasapiAudioSource {
    pub fn new() -> Self {
        Self
    }
}

impl AudioSource for WasapiAudioSource {
    fn open_system_audio(&mut self) -> Result<Box<dyn AudioStream>, PlatformError> {
        Ok(Box::new(LoopbackStream::open()?))
    }
}

/// 正在回环采集的默认输出设备。字段按释放顺序排列：先停止采集，最后离开 COM。
struct LoopbackStream {
    capture: IAudioCaptureClient,
    client: IAudioClient,
    _com: ComApartment,
}

impl LoopbackStream {
    fn open() -> Result<Self, PlatformError> {
        let com = ComApartment::enter()?;
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| win_error("无法枚举音频设备", e))?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| win_error("没有可用的音频输出设备", e))?;
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| win_error("无法打开音频输出设备", e))?;
            let block_align = FORMAT.channels * 4;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
                nChannels: FORMAT.channels,
                nSamplesPerSec: FORMAT.sample_rate,
                nAvgBytesPerSec: FORMAT.sample_rate * u32::from(block_align),
                nBlockAlign: block_align,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    BUFFER_DURATION,
                    0,
                    &format,
                    None,
                )
                .map_err(|e| win_error("无法采集系统声音", e))?;
            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| win_error("无法采集系统声音", e))?;
            client
                .Start()
                .map_err(|e| win_error("无法开始采集系统声音", e))?;
            Ok(Self {
                capture,
                client,
                _com: com,
            })
        }
    }
}

impl AudioStream for LoopbackStream {
    fn format(&self) -> AudioFormat {
        FORMAT
    }

    fn read(&mut self) -> Result<Vec<AudioChunk>, PlatformError> {
        let channels = usize::from(FORMAT.channels);
        let mut chunks = Vec::new();
        unsafe {
            loop {
                let size = self
                    .capture
                    .GetNextPacketSize()
                    .map_err(|e| win_error("读取系统声音失败", e))?;
                if size == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let (mut frames, mut flags) = (0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(|e| win_error("读取系统声音失败", e))?;
                let count = frames as usize * channels;
                let samples =
                    if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                        vec![0.0; count]
                    } else {
                        std::slice::from_raw_parts(data.cast::<f32>(), count).to_vec()
                    };
                self.capture
                    .ReleaseBuffer(frames)
                    .map_err(|e| win_error("读取系统声音失败", e))?;
                chunks.push(AudioChunk {
                    captured_at: Duration::ZERO,
                    samples,
                });
            }
        }
        // 最后一个包采集到此刻为止，之前的包首尾相接。
        let mut end = WindowsClock::new().monotonic();
        for chunk in chunks.iter_mut().rev() {
            let frames = (chunk.samples.len() / channels) as u32;
            end = end.saturating_sub(Duration::from_secs(1) * frames / FORMAT.sample_rate);
            chunk.captured_at = end;
        }
        Ok(chunks)
    }
}

impl Drop for LoopbackStream {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}
