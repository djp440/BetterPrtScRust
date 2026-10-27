//! 录屏用的音频：采集系统声音（默认输出设备正在播放的声音）和麦克风。
//!
//! 分工和视频一样：平台只给出带采集时间的 PCM 样本块；`core` 按采集时间把各路声音排进录屏的
//! 时间轴（去掉暂停的时间、补上没有声音时的空白）、混成一路，再把连续的样本交给
//! [`VideoWriter::write_audio`](crate::VideoWriter::write_audio)。

use std::time::Duration;

use crate::PlatformError;

/// PCM 音频的格式：样本都是交错排列的 `f32`（-1.0 ～ 1.0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    /// 采样率（每秒采样帧数）。
    pub sample_rate: u32,
    /// 声道数；一个采样帧含每个声道各一个样本。
    pub channels: u16,
}

/// 采集到的一段连续的声音。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    /// 第一个采样帧被采集的时刻，与 [`crate::Clock::monotonic`] 同一时间基准。
    pub captured_at: Duration,
    /// 交错排列的样本，长度是声道数的整数倍。
    pub samples: Vec<f32>,
}

/// 一个音频输入设备（麦克风）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// 设备的标识，重启后不变；配置文件里记的是它。
    pub id: String,
    /// 给用户看的名字。
    pub name: String,
}

/// 音频源。
pub trait AudioSource {
    /// 采集到的声音的格式：这个音频源打开的每一路流都是这个格式（平台负责转换），
    /// 所以 `core` 可以直接把它们混在一起。
    fn format(&self) -> AudioFormat;

    /// 开始采集系统声音（默认输出设备的回环）。没有可用的输出设备时返回错误。
    /// 返回的流被丢弃时停止采集。
    fn open_system_audio(&mut self) -> Result<Box<dyn AudioStream>, PlatformError>;

    /// 开始采集麦克风：`device` 是 [`AudioDevice::id`]，`None` 表示系统默认的输入设备。
    /// 设备不存在或打不开时返回错误。返回的流被丢弃时停止采集。
    fn open_microphone(
        &mut self,
        device: Option<&str>,
    ) -> Result<Box<dyn AudioStream>, PlatformError>;
}

/// 一路正在采集的声音。
pub trait AudioStream {
    /// 取出上次读取以来采集到的声音，按采集时间排列。系统没有播放声音时可能什么也没有
    /// （回环采集在静音期间不产生数据）。需要定期读取（例如每个节拍），否则平台的缓冲会溢出、丢掉声音。
    fn read(&mut self) -> Result<Vec<AudioChunk>, PlatformError>;
}
