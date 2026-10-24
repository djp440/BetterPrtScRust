/// 本地时区的日期时间（墙上时间），精确到毫秒。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LocalDateTime {
    pub year: u16,
    /// 1–12
    pub month: u8,
    /// 1–31
    pub day: u8,
    /// 0–23
    pub hour: u8,
    /// 0–59
    pub minute: u8,
    /// 0–59
    pub second: u8,
    /// 0–999
    pub millisecond: u16,
}

impl LocalDateTime {
    /// 精确到秒的日期时间，毫秒为 0。
    pub const fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millisecond: 0,
        }
    }

    pub const fn with_millisecond(mut self, millisecond: u16) -> Self {
        self.millisecond = millisecond;
        self
    }
}

/// 时钟：提供当前的本地时间，例如用于命名模板里的日期时间。
pub trait Clock {
    fn now(&self) -> LocalDateTime;
}
