/// 系统的界面语言。
pub trait SystemLanguage {
    /// 用户偏好的界面语言，按优先级排列，形如 `zh-CN`、`en-US`（BCP 47）。
    fn preferred_ui_languages(&self) -> Vec<String>;
}
