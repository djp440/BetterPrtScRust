//! 产物的输出选项：输出格式、自动存储目录、命名模板。

use std::path::PathBuf;

use bps_platform::ClipboardItem;

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
    /// 全部格式，按另存为对话框里列出的顺序。
    pub const ALL: [OutputFormat; 4] = [
        OutputFormat::Png,
        OutputFormat::Jpeg,
        OutputFormat::WebP,
        OutputFormat::Avif,
    ];

    /// 文件扩展名（小写、不含点）；第一个是保存时用的，其余是同一格式的别名。
    /// 配置文件里 `output_format` 的写法也是这些。
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            OutputFormat::Png => &["png"],
            OutputFormat::Jpeg => &["jpg", "jpeg"],
            OutputFormat::WebP => &["webp"],
            OutputFormat::Avif => &["avif"],
        }
    }

    /// 保存时用的文件扩展名（不含点）。
    pub fn extension(self) -> &'static str {
        self.extensions()[0]
    }

    /// 格式的显示名，如另存为对话框的文件类型。
    pub fn display_name(self) -> &'static str {
        match self {
            OutputFormat::Png => "PNG",
            OutputFormat::Jpeg => "JPEG",
            OutputFormat::WebP => "WebP",
            OutputFormat::Avif => "AVIF",
        }
    }

    /// 编码是否很慢（AVIF 编码 4K 图约 0.9 秒）：慢格式的编码要放到后台线程，不能卡住事件循环。
    pub fn encodes_slowly(self) -> bool {
        matches!(self, OutputFormat::Avif)
    }

    /// 按扩展名（不区分大小写，不含点）找格式。
    pub fn from_extension(name: &str) -> Option<OutputFormat> {
        let name = name.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|format| format.extensions().contains(&name.as_str()))
    }

    /// 放进剪贴板的图像项：编码好的 `bytes` 对应的格式。
    pub(crate) fn clipboard_item(self, bytes: Vec<u8>) -> ClipboardItem {
        match self {
            OutputFormat::Png => ClipboardItem::Png(bytes),
            OutputFormat::Jpeg => ClipboardItem::Jpeg(bytes),
            OutputFormat::WebP => ClipboardItem::WebP(bytes),
            OutputFormat::Avif => ClipboardItem::Avif(bytes),
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
