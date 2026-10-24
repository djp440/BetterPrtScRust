//! 配置文件等数据的存放位置，常驻实例、设置界面和 `bps` 共用同一套规则。

/// 程序目录下有 `portable` 标记文件时用便携模式，否则用 `%APPDATA%\BetterPrtSc`。
pub fn data_location() -> Result<bps_core::DataLocation, bps_platform::PlatformError> {
    use bps_core::{DataLocation, PORTABLE_MARKER};
    use bps_platform::PlatformError;

    let exe = std::env::current_exe()
        .map_err(|e| PlatformError::new(format!("无法确定程序所在目录：{e}")))?;
    let program_dir = exe
        .parent()
        .ok_or_else(|| PlatformError::new("无法确定程序所在目录"))?;
    let portable = program_dir.join(PORTABLE_MARKER).is_file();
    let app_data = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    DataLocation::resolve(program_dir, portable, app_data.as_deref()).ok_or_else(|| {
        PlatformError::new(format!(
            "找不到 %APPDATA%；可以在程序目录下放一个名为 {PORTABLE_MARKER} 的文件，改用便携模式"
        ))
    })
}
