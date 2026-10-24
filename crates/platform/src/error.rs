use std::fmt;

/// 平台能力调用失败。
///
/// 只携带一条给人看的说明；`core` 不根据平台错误的细节做分支判断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformError {
    message: String,
}

impl PlatformError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PlatformError {}
