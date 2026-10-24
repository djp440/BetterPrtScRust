use serde::Deserialize;

/// 输出格式：产物写入磁盘时采用的文件格式。
///
/// 配置文件里写作 `png`、`jpg`（或 `jpeg`）、`webp`、`avif`、`mp4`、`gif`。
/// 截图使用前四种，录屏使用后两种；录屏的输出格式在开始录制前就已确定。
///
/// 这里是 #6 解析配置所需的最小定义；编码与写盘由自动存储（#5）和录屏（#18、#20）实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Png,
    #[serde(alias = "jpeg")]
    Jpg,
    Webp,
    Avif,
    Mp4,
    Gif,
}
