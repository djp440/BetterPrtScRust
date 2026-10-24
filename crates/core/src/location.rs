use std::path::{Path, PathBuf};

/// 便携模式的标记文件名：程序目录下有这个文件时以便携模式运行。
pub const PORTABLE_MARKER: &str = "portable";

/// 配置文件和日志的存放位置。
///
/// 便携模式下是程序所在目录，不写入用户目录；否则是 `%APPDATA%\BetterPrtSc`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLocation {
    dir: PathBuf,
    portable: bool,
}

impl DataLocation {
    /// 按便携模式规则决定存放位置。
    ///
    /// - `program_dir`：`BetterPrtSc.exe` 所在目录；
    /// - `portable_marker_exists`：`program_dir` 下是否有 [`PORTABLE_MARKER`] 文件；
    /// - `roaming_app_data`：`%APPDATA%`。不是便携模式又没有它时返回 `None`。
    pub fn resolve(
        program_dir: &Path,
        portable_marker_exists: bool,
        roaming_app_data: Option<&Path>,
    ) -> Option<DataLocation> {
        if portable_marker_exists {
            return Some(DataLocation {
                dir: program_dir.to_path_buf(),
                portable: true,
            });
        }
        roaming_app_data.map(|app_data| DataLocation {
            dir: app_data.join("BetterPrtSc"),
            portable: false,
        })
    }

    pub fn is_portable(&self) -> bool {
        self.portable
    }

    /// 配置文件 `config.toml` 的路径。
    pub fn config_file(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    /// 日志目录。
    pub fn log_dir(&self) -> PathBuf {
        self.dir.join("logs")
    }
}
