use std::any::Any;

/// 后台任务交回的结果；类型对平台不透明，由提交任务的一方（`core`）自己还原。
pub type BackgroundResult = Box<dyn Any + Send>;

/// 在后台线程做耗时工作（例如 AVIF 编码），不阻塞平台事件循环，让热键等事件及时响应。
///
/// 任务按提交顺序依次执行（同一时刻最多一个在跑，避免几份大图同时编码占用内存）；
/// 每个任务完成时，平台经事件循环递交一个 [`crate::PlatformEvent::BackgroundJobDone`]，
/// 收到后用 [`take_finished`](Self::take_finished) 取走结果，在事件循环线程上继续后续步骤。
pub trait BackgroundWorker {
    /// 提交一个任务。
    fn spawn(&mut self, job: Box<dyn FnOnce() -> BackgroundResult + Send>);

    /// 取走已经完成的任务的结果，按完成顺序排列。
    fn take_finished(&mut self) -> Vec<BackgroundResult>;
}
