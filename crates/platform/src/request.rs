use std::fmt;

/// 另一个进程（`bps` CLI、再次启动的 `BetterPrtSc.exe`）转交给常驻实例的一条请求。
///
/// 平台只负责搬运文本：消息的含义由 `core` 解析，`core` 处理完后调用
/// [`reply`](Self::reply) 把回复文本交还给发起方。请求被丢弃而没有回复时，
/// 发起方会得到“常驻实例没有回复”的错误。
pub struct IncomingRequest {
    message: String,
    reply: Box<dyn FnOnce(String) + Send>,
}

impl IncomingRequest {
    /// `reply` 在常驻实例回复时被调用一次，负责把回复送回发起方。
    pub fn new(message: impl Into<String>, reply: impl FnOnce(String) + Send + 'static) -> Self {
        Self {
            message: message.into(),
            reply: Box::new(reply),
        }
    }

    /// 发起方发来的请求文本。
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 把回复文本交还给发起方。
    pub fn reply(self, response: String) {
        (self.reply)(response)
    }
}

impl fmt::Debug for IncomingRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IncomingRequest")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}
