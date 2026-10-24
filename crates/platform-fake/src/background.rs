use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bps_platform::{BackgroundResult, BackgroundWorker};

type Job = Box<dyn FnOnce() -> BackgroundResult + Send>;

#[derive(Default)]
pub(crate) struct BackgroundState {
    /// 已提交、还没有运行的任务。
    pub pending: VecDeque<Job>,
    /// 已经运行完、还没被取走的结果。
    pub finished: VecDeque<BackgroundResult>,
}

/// 假的后台线程：任务只记下，不自己运行；由测试用 `FakePlatform::run_background_jobs`
/// 让它们依次运行（扮演后台线程），并取回平台会递交的完成事件。
pub(crate) struct FakeBackground(pub Arc<Mutex<BackgroundState>>);

impl BackgroundWorker for FakeBackground {
    fn spawn(&mut self, job: Job) {
        self.0.lock().unwrap().pending.push_back(job);
    }

    fn take_finished(&mut self) -> Vec<BackgroundResult> {
        self.0.lock().unwrap().finished.drain(..).collect()
    }
}
