use std::path::Path;

use crate::PlatformError;

/// 配置文件（`config.toml`）的存储与变更监视。
///
/// 文件放在哪里（`%APPDATA%` 或便携模式下的程序目录）由创建者决定；
/// 内容的格式、默认值和重新加载规则都由 `core` 负责。
pub trait ConfigFile {
    /// 配置文件的位置，用于提示用户和打开文件。
    fn path(&self) -> &Path;

    /// 读取配置文件的全文；文件不存在时返回 `Ok(None)`。
    fn read(&self) -> Result<Option<String>, PlatformError>;

    /// 用 `text` 替换配置文件的全文，必要时先创建所在目录。
    fn write(&mut self, text: &str) -> Result<(), PlatformError>;

    /// 开始监视外部修改：文件被修改、替换或删除后，平台事件循环递交
    /// [`crate::PlatformEvent::ConfigFileChanged`]。
    fn watch(&mut self) -> Result<(), PlatformError>;
}
