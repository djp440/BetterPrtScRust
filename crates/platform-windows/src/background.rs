//! 后台线程：把耗时的编码等工作挪出事件循环线程。
//!
//! 每个任务在自己的短命线程里运行，运行完线程就退出，常驻时不留线程；任务之间用一把锁
//! 串行化（同一时刻最多一个在跑），避免几份大图同时编码占用内存。结果放进共享队列，
//! 并经 [`EventSender`] 通知事件循环。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use bps_platform::{BackgroundResult, BackgroundWorker, PlatformEvent};

use crate::EventSender;

type Job = Box<dyn FnOnce() -> BackgroundResult + Send>;

/// 后台线程能力的 Windows 实现。
pub struct ThreadBackground {
    events: EventSender,
    /// 串行执行任务用的锁。
    serial: Arc<Mutex<()>>,
    finished: Arc<Mutex<VecDeque<BackgroundResult>>>,
}

impl ThreadBackground {
    /// 任务完成以 [`PlatformEvent::BackgroundJobDone`] 投递给 `events` 所属的事件循环。
    pub fn new(events: EventSender) -> Self {
        Self {
            events,
            serial: Arc::default(),
            finished: Arc::default(),
        }
    }
}

impl BackgroundWorker for ThreadBackground {
    fn spawn(&mut self, job: Job) {
        // 线程启动失败时任务随闭包丢失，所以先放进槽里，失败了还能在当前线程上运行。
        let slot = Arc::new(Mutex::new(Some(job)));
        let run = {
            let (slot, serial, finished, events) = (
                slot.clone(),
                self.serial.clone(),
                self.finished.clone(),
                self.events.clone(),
            );
            move || {
                let _turn = serial.lock().unwrap_or_else(PoisonError::into_inner);
                let job = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
                if let Some(job) = job {
                    let result = job();
                    finished
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push_back(result);
                    let _ = events.send(PlatformEvent::BackgroundJobDone);
                }
            }
        };
        let fallback = run.clone();
        if thread::Builder::new()
            .name("bps-background".to_owned())
            .spawn(run)
            .is_err()
        {
            fallback();
        }
    }

    fn take_finished(&mut self) -> Vec<BackgroundResult> {
        self.finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}
