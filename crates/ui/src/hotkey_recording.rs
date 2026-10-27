//! 设置界面里的“录制热键”会话：装上按键钩子、暂停常驻实例的全局热键，
//! 直到录到结论并且所有键都松开（或超时、被取消）。

use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bps_core::{HotkeyRecorder, RecordingOutcome};
use bps_platform::{Hotkey, KeyInput, Modifiers};

/// 一次录制没有结论时自动取消的时间。
const RECORD_TIMEOUT: Duration = Duration::from_secs(30);
/// 有了结论后，等所有键松开的最长时间（钩子在此期间继续吞掉按键，免得残留的松开事件泄漏）。
const RELEASE_TIMEOUT: Duration = Duration::from_secs(2);
/// 续租暂停的间隔（常驻实例的租约是 4 秒）。
const RENEW_INTERVAL: Duration = Duration::from_secs(1);

/// 一路按键事件；丢弃时停止捕获（卸载钩子）。
pub struct KeyStream {
    pub inputs: Receiver<KeyInput>,
    /// 保持捕获存活的守卫，丢弃即停止。
    pub guard: Box<dyn Any>,
}

/// 开始捕获按键；失败时给出说明（例如钩子装不上）。
pub type StartKeyCapture = Arc<dyn Fn() -> Result<KeyStream, String> + Send + Sync>;

/// 暂停（`true`，同时续租）或恢复（`false`）常驻实例的全局热键。在后台线程调用。
pub type PauseHotkeys = Arc<dyn Fn(bool) + Send + Sync>;

/// 录制期间的暂停：后台线程每秒续租，丢弃时立即恢复。
/// 设置进程异常退出来不及恢复时，常驻实例的租约到期后自行恢复。
struct PauseLease {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl PauseLease {
    fn start(pause: PauseHotkeys) -> Self {
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("bps-pause-hotkeys".to_owned())
            .spawn(move || {
                loop {
                    pause(true);
                    // 发送端被丢弃（或发来消息）就结束。
                    if !matches!(
                        stopped.recv_timeout(RENEW_INTERVAL),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
                pause(false);
            })
            .ok();
        Self {
            stop: Some(stop),
            thread,
        }
    }
}

impl Drop for PauseLease {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 正在进行的一次录制。
pub struct RecordingSession {
    /// 录的是第几条绑定。
    pub binding: usize,
    stream: KeyStream,
    recorder: HotkeyRecorder,
    started: Instant,
    /// 有结论的时刻。
    finished_at: Option<Instant>,
    /// 最近按了热键不支持的键。
    pub unsupported: bool,
    /// 正按住的修饰键，界面实时预览用。
    pub preview: Modifiers,
    _pause: PauseLease,
}

/// 录制会话的进展。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// 还在等主键；`Modifiers` 是正按住的修饰键。
    Waiting(Modifiers),
    /// 有结论，界面要采用它。
    Concluded(Conclusion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conclusion {
    Recorded(Hotkey),
    Cleared,
    Cancelled,
}

impl RecordingSession {
    /// 开始录制第 `binding` 条绑定的热键。
    pub fn start(
        binding: usize,
        start_capture: &StartKeyCapture,
        pause: PauseHotkeys,
    ) -> Result<Self, String> {
        let stream = start_capture()?;
        Ok(Self {
            binding,
            stream,
            recorder: HotkeyRecorder::new(),
            started: Instant::now(),
            finished_at: None,
            unsupported: false,
            preview: Modifiers::NONE,
            _pause: PauseLease::start(pause),
        })
    }

    /// 处理收到的按键；有结论时返回它（只返回一次）。
    pub fn poll(&mut self) -> Progress {
        let mut modifiers = self.recorder.modifiers();
        while let Ok(input) = self.stream.inputs.try_recv() {
            match self.recorder.feed(input) {
                RecordingOutcome::Waiting(m) => {
                    modifiers = m;
                    self.unsupported = false;
                }
                RecordingOutcome::Unsupported => self.unsupported = true,
                RecordingOutcome::Recorded(hotkey) => {
                    return self.conclude(Conclusion::Recorded(hotkey));
                }
                RecordingOutcome::Cleared => return self.conclude(Conclusion::Cleared),
                RecordingOutcome::Cancelled => return self.conclude(Conclusion::Cancelled),
                RecordingOutcome::Finished => {}
            }
        }
        if self.started.elapsed() >= RECORD_TIMEOUT {
            return self.conclude(Conclusion::Cancelled);
        }
        self.preview = modifiers;
        Progress::Waiting(modifiers)
    }

    fn conclude(&mut self, conclusion: Conclusion) -> Progress {
        self.finished_at = Some(Instant::now());
        Progress::Concluded(conclusion)
    }

    /// 有了结论之后，等所有键松开（或超时）才可以结束会话。
    pub fn drain_finished(&mut self) -> bool {
        while let Ok(input) = self.stream.inputs.try_recv() {
            self.recorder.feed(input);
        }
        self.recorder.all_released()
            || self
                .finished_at
                .is_some_and(|at| at.elapsed() >= RELEASE_TIMEOUT)
    }

    pub fn has_concluded(&self) -> bool {
        self.finished_at.is_some()
    }
}
