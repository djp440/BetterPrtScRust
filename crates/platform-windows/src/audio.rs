//! 录屏的声音：WASAPI 共享模式下回环采集默认输出设备（系统声音，“正在播放的声音”），
//! 或者采集麦克风（系统默认的录音设备或指定的设备）。
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

use bps_platform::{
    AudioChunk, AudioDevice, AudioFormat, AudioSource, AudioStream, Clock, PlatformError,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE,
    IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    WAVEFORMATEX, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::core::{HSTRING, PWSTR};

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

/// 录屏的音频源：默认输出设备的 WASAPI 回环，以及麦克风。
#[derive(Debug, Default)]
pub struct WasapiAudioSource;

impl WasapiAudioSource {
    pub fn new() -> Self {
        Self
    }
}

impl AudioSource for WasapiAudioSource {
    fn format(&self) -> AudioFormat {
        FORMAT
    }

    fn open_system_audio(&mut self) -> Result<Box<dyn AudioStream>, PlatformError> {
        Ok(Box::new(CaptureStream::open(Input::System, None)?))
    }

    fn open_microphone(
        &mut self,
        device: Option<&str>,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        Ok(Box::new(CaptureStream::open(Input::Microphone, device)?))
    }
}

/// 系统里可用的麦克风（设备标识和名字），按系统给出的顺序。
pub fn microphones() -> Result<Vec<AudioDevice>, PlatformError> {
    let _com = ComApartment::enter()?;
    unsafe {
        let collection = enumerator()?
            .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
            .map_err(|e| win_error("无法列出麦克风", e))?;
        let count = collection
            .GetCount()
            .map_err(|e| win_error("无法列出麦克风", e))?;
        let mut devices = Vec::new();
        for index in 0..count {
            let device = collection
                .Item(index)
                .map_err(|e| win_error("无法列出麦克风", e))?;
            let id = take_string(device.GetId().map_err(|e| win_error("无法列出麦克风", e))?);
            // 读不到名字时用设备标识代替，不影响选择。
            let name = friendly_name(&device).unwrap_or_else(|| id.clone());
            devices.push(AudioDevice { id, name });
        }
        Ok(devices)
    }
}

/// 设备的名字（如“麦克风 (Realtek High Definition Audio)”）。
unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let name = take_string(PropVariantToStringAlloc(&value).ok()?);
        (!name.is_empty()).then_some(name)
    }
}

/// 转成 `String` 并释放 COM 分配的字符串。
unsafe fn take_string(text: PWSTR) -> String {
    unsafe {
        let string = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0.cast()));
        string
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator, PlatformError> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|e| win_error("无法枚举音频设备", e))
}

/// 采集的是哪一路声音。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    System,
    Microphone,
}

impl Input {
    /// 出错时说的“采集什么”。
    fn noun(self) -> &'static str {
        match self {
            Input::System => "系统声音",
            Input::Microphone => "麦克风",
        }
    }
}

/// 正在采集的设备。字段按释放顺序排列：先停止采集，最后离开 COM。
struct CaptureStream {
    capture: IAudioCaptureClient,
    client: IAudioClient,
    input: Input,
    _com: ComApartment,
}

impl CaptureStream {
    /// 打开 `input`；麦克风给了 `device` 时打开这个设备，否则打开系统默认的录音设备。
    fn open(input: Input, device: Option<&str>) -> Result<Self, PlatformError> {
        let com = ComApartment::enter()?;
        let noun = input.noun();
        unsafe {
            let enumerator = enumerator()?;
            let device = match (input, device) {
                (Input::System, _) => enumerator
                    .GetDefaultAudioEndpoint(eRender, eConsole)
                    .map_err(|e| win_error("没有可用的音频输出设备", e))?,
                (Input::Microphone, None) => enumerator
                    .GetDefaultAudioEndpoint(eCapture, eConsole)
                    .map_err(|e| win_error("没有可用的麦克风", e))?,
                (Input::Microphone, Some(id)) => enumerator
                    .GetDevice(&HSTRING::from(id))
                    .map_err(|e| win_error("找不到选中的麦克风", e))?,
            };
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| win_error(&format!("无法打开{noun}设备"), e))?;
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
            let mut flags =
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            if input == Input::System {
                flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
            }
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    flags,
                    BUFFER_DURATION,
                    0,
                    &format,
                    None,
                )
                .map_err(|e| win_error(&format!("无法采集{noun}"), e))?;
            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| win_error(&format!("无法采集{noun}"), e))?;
            client
                .Start()
                .map_err(|e| win_error(&format!("无法开始采集{noun}"), e))?;
            Ok(Self {
                capture,
                client,
                input,
                _com: com,
            })
        }
    }
}

impl AudioStream for CaptureStream {
    fn read(&mut self) -> Result<Vec<AudioChunk>, PlatformError> {
        let channels = usize::from(FORMAT.channels);
        let failed = |e| win_error(&format!("读取{}失败", self.input.noun()), e);
        let mut chunks = Vec::new();
        unsafe {
            loop {
                let size = self.capture.GetNextPacketSize().map_err(failed)?;
                if size == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let (mut frames, mut flags) = (0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(failed)?;
                let count = frames as usize * channels;
                let samples =
                    if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                        vec![0.0; count]
                    } else {
                        std::slice::from_raw_parts(data.cast::<f32>(), count).to_vec()
                    };
                self.capture.ReleaseBuffer(frames).map_err(failed)?;
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

impl Drop for CaptureStream {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}
