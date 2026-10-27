//! 录屏用的音频：采集系统声音（默认输出设备正在播放的声音）。
//!
//! 分工和视频一样：平台只给出带采集时间的 PCM 样本块；`core` 按采集时间把它们排进录屏的
//! 时间轴（去掉暂停的时间、补上没有声音时的空白），再把连续的样本交给
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

/// 音频源。
pub trait AudioSource {
    /// 开始采集系统声音（默认输出设备的回环）。没有可用的输出设备时返回错误。
    /// 返回的流被丢弃时停止采集。
    fn open_system_audio(&mut self) -> Result<Box<dyn AudioStream>, PlatformError>;
}

/// 一路正在采集的声音。
pub trait AudioStream {
    /// 样本的格式，打开后不变。
    fn format(&self) -> AudioFormat;

    /// 取出上次读取以来采集到的声音，按采集时间排列。系统没有播放声音时可能什么也没有
    /// （回环采集在静音期间不产生数据）。需要定期读取（例如每个节拍），否则平台的缓冲会溢出、丢掉声音。
    fn read(&mut self) -> Result<Vec<AudioChunk>, PlatformError>;
}
