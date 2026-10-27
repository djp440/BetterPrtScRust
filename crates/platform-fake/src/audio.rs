//! 假音频源：测试用 [`crate::FakePlatform::play_audio`]（系统声音）、
//! [`crate::FakePlatform::speak`]（麦克风）“播放”一段声音，打开着的对应音频流在下一次
//! `read` 时把它原样交出，采集时间是播放时的单调时间。格式固定为 [`FAKE_AUDIO_FORMAT`]
//! （每秒 1000 帧、单声道：一个样本就是 1 毫秒，便于写期望值）。

use std::sync::{Arc, Mutex};

use bps_platform::{AudioChunk, AudioDevice, AudioFormat, AudioSource, AudioStream, PlatformError};

/// 假音频源的格式。
pub const FAKE_AUDIO_FORMAT: AudioFormat = AudioFormat {
    sample_rate: 1000,
    channels: 1,
};

/// 一路声音（系统声音或麦克风）的假状态。
#[derive(Default)]
pub(crate) struct InputState {
    /// 打开失败的原因（例如没有可用的设备）。
    pub unavailable: Option<String>,
    /// 还没有关闭的音频流数量。
    pub open_streams: usize,
    /// 播放了、还没被读走的声音。
    pub pending: Vec<AudioChunk>,
    /// 读取声音时失败的原因（例如设备被拔掉）。
    pub read_error: Option<String>,
}

pub(crate) struct AudioState {
    pub system: InputState,
    pub microphone: InputState,
    /// 系统里的麦克风；默认只有一个 [`DEFAULT_MICROPHONE`]。
    pub microphones: Vec<AudioDevice>,
    /// 最近一次成功打开的麦克风（`None` 表示系统默认设备）。
    pub opened_microphone: Option<Option<String>>,
}

/// 假平台默认的麦克风。
pub const DEFAULT_MICROPHONE: &str = "fake-microphone";

impl Default for AudioState {
    fn default() -> Self {
        Self {
            system: InputState::default(),
            microphone: InputState::default(),
            microphones: vec![AudioDevice {
                id: DEFAULT_MICROPHONE.to_owned(),
                name: "假麦克风".to_owned(),
            }],
            opened_microphone: None,
        }
    }
}

/// 音频流采集的是哪一路。
#[derive(Clone, Copy)]
enum Input {
    System,
    Microphone,
}

impl AudioState {
    fn input(&mut self, input: Input) -> &mut InputState {
        match input {
            Input::System => &mut self.system,
            Input::Microphone => &mut self.microphone,
        }
    }
}

pub(crate) struct FakeAudioSource(pub Arc<Mutex<AudioState>>);

impl FakeAudioSource {
    fn open(&self, input: Input) -> Result<Box<dyn AudioStream>, PlatformError> {
        let mut state = self.0.lock().unwrap();
        let state = state.input(input);
        if let Some(reason) = &state.unavailable {
            return Err(PlatformError::new(reason.clone()));
        }
        state.open_streams += 1;
        // 打开之前播放的声音采集不到。
        state.pending.clear();
        Ok(Box::new(FakeAudioStream {
            state: self.0.clone(),
            input,
        }))
    }
}

impl AudioSource for FakeAudioSource {
    fn format(&self) -> AudioFormat {
        FAKE_AUDIO_FORMAT
    }

    fn open_system_audio(&mut self) -> Result<Box<dyn AudioStream>, PlatformError> {
        self.open(Input::System)
    }

    fn open_microphone(
        &mut self,
        device: Option<&str>,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        {
            let state = self.0.lock().unwrap();
            if let Some(id) = device
                && !state.microphones.iter().any(|m| m.id == id)
            {
                return Err(PlatformError::new(format!("找不到麦克风 {id}")));
            }
            if device.is_none() && state.microphones.is_empty() {
                return Err(PlatformError::new("没有可用的麦克风"));
            }
        }
        let stream = self.open(Input::Microphone)?;
        self.0.lock().unwrap().opened_microphone = Some(device.map(str::to_owned));
        Ok(stream)
    }
}

struct FakeAudioStream {
    state: Arc<Mutex<AudioState>>,
    input: Input,
}

impl AudioStream for FakeAudioStream {
    fn read(&mut self) -> Result<Vec<AudioChunk>, PlatformError> {
        let mut state = self.state.lock().unwrap();
        let state = state.input(self.input);
        if let Some(reason) = &state.read_error {
            return Err(PlatformError::new(reason.clone()));
        }
        Ok(std::mem::take(&mut state.pending))
    }
}

impl Drop for FakeAudioStream {
    fn drop(&mut self) {
        self.state.lock().unwrap().input(self.input).open_streams -= 1;
    }
}
