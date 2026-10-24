use std::path::PathBuf;

use crate::PlatformError;

/// 文件对话框：让用户自己选择位置和文件名（“另存为”）。
pub trait FileDialog {
    /// 弹出“另存为”对话框，阻塞到用户关闭它；返回用户选择的完整路径，用户取消时返回 `None`。
    ///
    /// 选区遮罩显示着时，对话框显示在遮罩之上。用户选择已存在的文件时，对话框已经请用户
    /// 确认过覆盖，调用方可以直接覆盖它。
    fn save_as(&mut self, options: &SaveAsOptions) -> Result<Option<PathBuf>, PlatformError>;
}

/// “另存为”对话框的初始内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAsOptions {
    /// 建议的文件名（含扩展名，不含目录）。
    pub file_name: String,
    /// 系统没有记住上一次另存为的位置时打开的目录；`None` 由系统决定。
    pub folder: Option<PathBuf>,
    /// 可选的文件类型。
    pub file_types: Vec<FileType>,
    /// 默认选中的文件类型在 `file_types` 中的下标。
    pub default_file_type: usize,
}

/// 对话框里的一种文件类型，例如“PNG 图像（*.png）”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileType {
    /// 给人看的名称。
    pub name: String,
    /// 扩展名（不含点，小写），第一个是这种类型的默认扩展名。
    pub extensions: Vec<String>,
}
