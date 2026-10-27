use bps_platform::{Autostart, PlatformError};

/// 卸载前的清理：移除指向本程序的全部开机自启登记，别处另一份 BetterPrtSc 的登记不动。
///
/// 某种方式移除失败（例如没有管理员权限移除不了 [`bps_platform::AutostartMethod::Elevated`]）
/// 不影响其他方式；有失败时返回汇总的错误。
pub fn remove_autostart(autostart: &mut dyn Autostart) -> Result<(), PlatformError> {
    let mut errors = Vec::new();
    for entry in autostart.entries()? {
        if entry.this_program
            && let Err(e) = autostart.disable(entry.method)
        {
            errors.push(e.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::new(errors.join("；")))
    }
}
