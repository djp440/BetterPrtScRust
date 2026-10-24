//! 卸载前的清理（安装包卸载时运行 `BetterPrtSc.exe --remove-autostart`）：
//! 移除指向本程序的开机自启登记，别处另一份 BetterPrtSc 的登记不动。

use bps_core::remove_autostart;
use bps_platform::{AutostartEntry, AutostartMethod};
use bps_platform_fake::FakePlatform;

#[test]
fn removes_both_methods_registered_for_this_program() {
    let fake = FakePlatform::new()
        .as_administrator()
        .with_autostart_entry(AutostartMethod::Normal)
        .with_autostart_entry(AutostartMethod::Elevated);
    let mut platform = fake.platform();

    remove_autostart(platform.autostart.as_mut()).unwrap();

    assert!(fake.autostart_entries().is_empty());
}

#[test]
fn leaves_registration_of_another_copy_alone() {
    let fake = FakePlatform::new().with_autostart_entry_of_other_program(AutostartMethod::Normal);
    let mut platform = fake.platform();

    remove_autostart(platform.autostart.as_mut()).unwrap();

    assert_eq!(
        fake.autostart_entries(),
        [AutostartEntry {
            method: AutostartMethod::Normal,
            this_program: false,
        }]
    );
}

#[test]
fn reports_failure_but_still_removes_what_it_can() {
    // 没有管理员权限时移除不了计划任务，但注册表 Run 项照样移除。
    let fake = FakePlatform::new()
        .with_autostart_entry(AutostartMethod::Normal)
        .with_autostart_entry(AutostartMethod::Elevated);
    let mut platform = fake.platform();

    let error = remove_autostart(platform.autostart.as_mut()).unwrap_err();

    assert!(error.message().contains("需要管理员权限"), "{error}");
    assert_eq!(
        fake.autostart_entries(),
        [AutostartEntry {
            method: AutostartMethod::Elevated,
            this_program: true,
        }]
    );
}
