//! 产物的输出选项：输出格式、自动存储目录、命名模板。

use std::path::PathBuf;

use crate::FilenameTemplate;

/// 截图写入磁盘（以及放进剪贴板）时采用的文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutputFormat {
    /// 无损，保留透明度。
    #[default]
    Png,
    /// 有损，不支持透明：透明部分合成到白色背景上。
    Jpeg,
    /// 无损 WebP，保留透明度。
    WebP,
    /// 有损 AVIF，保留透明度；编码明显慢于其他格式。
    Avif,
}

impl OutputFormat {
    /// 文件扩展名（不含点）。
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Png => "png",
            OutputFormat::Jpeg => "jpg",
            OutputFormat::WebP => "webp",
            OutputFormat::Avif => "avif",
        }
    }
}

/// 自动存储的目录。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SaveDir {
    /// 当前用户“图片”文件夹下的 `BetterPrtSc` 子目录。
    #[default]
    Default,
    /// 用户指定的目录。
    Custom(PathBuf),
}

/// 默认存储目录在“图片”文件夹下的子目录名。
pub(crate) const DEFAULT_SAVE_SUBDIR: &str = "BetterPrtSc";

/// 一次捕获的产物怎么输出。字段名与配置文件里的覆盖项一致。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputOptions {
    /// 自动存储的目录。
    pub save_dir: SaveDir,
    /// 自动存储时生成文件名的命名模板。
    pub filename_template: FilenameTemplate,
    /// 写入磁盘和剪贴板时的图像格式。
    pub output_format: OutputFormat,
}
