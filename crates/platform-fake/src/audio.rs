//! 假音频源：测试用 [`crate::FakePlatform::play_audio`] “播放”一段声音，打开着的音频流在下一次
//! `read` 时把它原样交出，采集时间是播放时的单调时间。格式固定为 [`FAKE_AUDIO_FORMAT`]
//! （每秒 1000 帧、单声道：一个样本就是 1 毫秒，便于写期望值）。

use std::sync::{Arc, Mutex};

use bps_platform::{AudioChunk, AudioFormat, AudioSource, AudioStream, PlatformError};

/// 假音频源的格式。
pub const FAKE_AUDIO_FORMAT: AudioFormat = AudioFormat {
    sample_rate: 1000,
    channels: 1,
};

#[derive(Default)]
pub(crate) struct AudioState {
    /// 没有可用的输出设备时打开失败的原因。
    pub unavailable: Option<String>,
    /// 还没有关闭的音频流数量。
    pub open_streams: usize,
    /// 播放了、还没被读走的声音。
    pub pending: Vec<AudioChunk>,
    /// 读取声音时失败的原因（例如设备被拔掉）。
    pub read_error: Option<String>,
}

pub(crate) struct FakeAudioSource(pub Arc<Mutex<AudioState>>);

impl AudioSource for FakeAudioSource {
    fn open_system_audio(&mut self) -> Result<Box<dyn AudioStream>, PlatformError> {
        let mut state = self.0.lock().unwrap();
        if let Some(reason) = &state.unavailable {
            return Err(PlatformError::new(reason.clone()));
        }
        state.open_streams += 1;
        // 打开之前播放的声音采集不到。
        state.pending.clear();
        Ok(Box::new(FakeAudioStream(self.0.clone())))
    }
}

struct FakeAudioStream(Arc<Mutex<AudioState>>);

impl AudioStream for FakeAudioStream {
    fn format(&self) -> AudioFormat {
        FAKE_AUDIO_FORMAT
    }

    fn read(&mut self) -> Result<Vec<AudioChunk>, PlatformError> {
        let mut state = self.0.lock().unwrap();
        if let Some(reason) = &state.read_error {
            return Err(PlatformError::new(reason.clone()));
        }
        Ok(std::mem::take(&mut state.pending))
    }
}

impl Drop for FakeAudioStream {
    fn drop(&mut self) {
        self.0.lock().unwrap().open_streams -= 1;
    }
}
