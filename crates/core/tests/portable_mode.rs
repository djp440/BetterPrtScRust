//! 便携模式：程序目录下有 `portable` 标记文件时，配置和日志都放在程序目录里，
//! 不写入 `%APPDATA%`。

use std::path::Path;

use bps_core::DataLocation;

const PROGRAM_DIR: &str = r"D:\Tools\BetterPrtSc";
const APP_DATA: &str = r"C:\Users\someone\AppData\Roaming";

#[test]
fn portable_marker_keeps_config_and_logs_in_program_directory() {
    let location =
        DataLocation::resolve(Path::new(PROGRAM_DIR), true, Some(Path::new(APP_DATA))).unwrap();

    assert!(location.is_portable());
    assert_eq!(
        location.config_file(),
        Path::new(r"D:\Tools\BetterPrtSc\config.toml")
    );
    assert_eq!(location.log_dir(), Path::new(r"D:\Tools\BetterPrtSc\logs"));
}

#[test]
fn without_portable_marker_config_and_logs_go_to_roaming_app_data() {
    let location =
        DataLocation::resolve(Path::new(PROGRAM_DIR), false, Some(Path::new(APP_DATA))).unwrap();

    assert!(!location.is_portable());
    assert_eq!(
        location.config_file(),
        Path::new(r"C:\Users\someone\AppData\Roaming\BetterPrtSc\config.toml")
    );
    assert_eq!(
        location.log_dir(),
        Path::new(r"C:\Users\someone\AppData\Roaming\BetterPrtSc\logs")
    );
}

#[test]
fn portable_mode_does_not_need_app_data() {
    assert!(DataLocation::resolve(Path::new(PROGRAM_DIR), true, None).is_some());
    assert!(DataLocation::resolve(Path::new(PROGRAM_DIR), false, None).is_none());
}
