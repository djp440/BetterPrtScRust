//! 检查更新（ADR 0004）：默认关闭，关闭时不发任何网络请求；开启后只查询 GitHub Releases 上的
//! 最新正式版本，有新版本时只提示（点击打开发布页面），不下载。

use bps_core::{ResidentInstance, UpdateStatus, VERSION};
use bps_platform::{LatestRelease, NotificationClick, PlatformError};
use bps_platform_fake::{FakePlatform, FakeResidentChannel};

const REPOSITORY: &str = "djp440/BetterPrtScRust";
const PAGE: &str = "https://github.com/djp440/BetterPrtScRust/releases/tag/v99.0.0";

const BINDING: &str = r#"
[[bindings]]
hotkey = "Ctrl+F9"
mode = "display"
after = "copy_to_clipboard"
"#;

fn config(check_for_updates: bool, language: &str) -> String {
    format!("check_for_updates = {check_for_updates}\nlanguage = \"{language}\"\n{BINDING}")
}

fn release(tag: &str) -> LatestRelease {
    LatestRelease {
        tag: tag.to_owned(),
        page_url: PAGE.to_owned(),
    }
}

/// 启动一个开启了检查更新的常驻实例。
fn started_with_update_check() -> (FakePlatform, ResidentInstance) {
    let fake = FakePlatform::new().with_config_file(&config(true, "zh-CN"));
    let instance = ResidentInstance::start(fake.platform());
    (fake, instance)
}

#[test]
fn update_check_is_off_by_default_and_never_sends_a_request() {
    let fake = FakePlatform::new();
    let mut instance = ResidentInstance::start(fake.platform());
    assert!(!instance.config().check_for_updates);

    // 打开设置、改配置、查询状态……都不会触发检查更新。
    instance
        .handle(fake.choose_tray_menu_item("设置").unwrap())
        .unwrap();
    let edited = fake.edit_config_file(&config(false, "en")).unwrap();
    instance.handle(edited).unwrap();
    let mut channel = FakeResidentChannel::running(move |event| {
        let _ = instance.handle(event);
    });
    let _ = bps_core::forward(
        &bps_core::Request::Status,
        &mut channel,
        bps_core::WhenNotRunning::GiveUp,
    );

    assert_eq!(fake.update_check_requests(), Vec::<String>::new());
}

#[test]
fn enabled_update_check_queries_github_releases_once_at_startup() {
    let (fake, instance) = started_with_update_check();

    assert_eq!(fake.update_check_requests(), [REPOSITORY]);
    assert_eq!(instance.status().update, UpdateStatus::Checking);
}

#[test]
fn newer_release_is_announced_with_a_link_to_its_page_without_downloading() {
    let (fake, mut instance) = started_with_update_check();
    let files_before = fake.files();

    let fetched = fake.finish_update_check(Ok(Some(release("v99.0.0"))));
    instance.handle(fetched.unwrap()).unwrap();

    let notification = fake.notifications().pop().unwrap();
    assert_eq!(notification.title, "发现新版本 v99.0.0");
    assert!(notification.body.contains(VERSION), "{notification:?}");
    assert_eq!(
        notification.on_click,
        Some(NotificationClick::OpenUrl(PAGE.to_owned()))
    );
    assert_eq!(
        instance.status().update,
        UpdateStatus::Available {
            version: "v99.0.0".to_owned(),
            page_url: PAGE.to_owned(),
        }
    );
    assert_eq!(fake.files(), files_before, "只提示，不下载");
    assert_eq!(fake.update_check_requests().len(), 1);
}

#[test]
fn same_older_or_missing_release_is_up_to_date_without_notification() {
    let current = format!("v{VERSION}");
    let current_prerelease = format!("v{VERSION}-rc.1");
    for latest in [
        Some(current.as_str()),
        Some(VERSION),
        Some("v0.0.1"),
        Some(current_prerelease.as_str()),
        None,
    ] {
        let (fake, mut instance) = started_with_update_check();
        let notifications_before = fake.notifications().len();

        let fetched = fake.finish_update_check(Ok(latest.map(release)));
        instance.handle(fetched.unwrap()).unwrap();

        assert_eq!(
            fake.notifications().len(),
            notifications_before,
            "{latest:?}"
        );
        assert_eq!(
            instance.status().update,
            UpdateStatus::UpToDate,
            "{latest:?}"
        );
    }
}

#[test]
fn failed_or_unrecognised_check_is_silent_but_shown_in_status() {
    let (fake, mut instance) = started_with_update_check();
    let fetched = fake.finish_update_check(Err(PlatformError::new("无法连接 api.github.com")));
    instance.handle(fetched.unwrap()).unwrap();
    assert!(fake.notifications().is_empty());
    assert!(
        matches!(&instance.status().update, UpdateStatus::Failed(m) if m.contains("api.github.com")),
        "{:?}",
        instance.status().update
    );

    let (fake, mut instance) = started_with_update_check();
    let fetched = fake.finish_update_check(Ok(Some(release("nightly"))));
    instance.handle(fetched.unwrap()).unwrap();
    assert!(fake.notifications().is_empty());
    assert!(
        matches!(&instance.status().update, UpdateStatus::Failed(m) if m.contains("nightly")),
        "{:?}",
        instance.status().update
    );
}

#[test]
fn turning_update_check_on_in_settings_queries_right_away_once() {
    let fake = FakePlatform::new().with_config_file(&config(false, "zh-CN"));
    let mut instance = ResidentInstance::start(fake.platform());
    assert!(fake.update_check_requests().is_empty());

    let enabled = fake.edit_config_file(&config(true, "zh-CN")).unwrap();
    instance.handle(enabled).unwrap();
    assert_eq!(fake.update_check_requests(), [REPOSITORY]);

    // 之后改别的设置不会再查。
    let edited = fake.edit_config_file(&config(true, "en")).unwrap();
    instance.handle(edited).unwrap();
    assert_eq!(fake.update_check_requests().len(), 1);
}

#[test]
fn turning_update_check_off_discards_the_pending_result() {
    let (fake, mut instance) = started_with_update_check();

    let disabled = fake.edit_config_file(&config(false, "zh-CN")).unwrap();
    instance.handle(disabled).unwrap();
    assert_eq!(instance.status().update, UpdateStatus::Disabled);

    let fetched = fake.finish_update_check(Ok(Some(release("v99.0.0"))));
    instance.handle(fetched.unwrap()).unwrap();

    assert!(fake.notifications().is_empty());
    assert_eq!(instance.status().update, UpdateStatus::Disabled);
}

#[test]
fn the_same_new_version_is_announced_only_once_per_run() {
    let (fake, mut instance) = started_with_update_check();
    let fetched = fake.finish_update_check(Ok(Some(release("v99.0.0"))));
    instance.handle(fetched.unwrap()).unwrap();
    assert_eq!(fake.notifications().len(), 1);

    for text in [config(false, "zh-CN"), config(true, "zh-CN")] {
        let event = fake.edit_config_file(&text).unwrap();
        instance.handle(event).unwrap();
    }
    let fetched = fake.finish_update_check(Ok(Some(release("v99.0.0"))));
    instance.handle(fetched.unwrap()).unwrap();

    assert_eq!(fake.update_check_requests().len(), 2);
    assert_eq!(fake.notifications().len(), 1);
}

#[test]
fn announcement_follows_the_interface_language() {
    let fake = FakePlatform::new().with_config_file(&config(true, "en"));
    let mut instance = ResidentInstance::start(fake.platform());

    let fetched = fake.finish_update_check(Ok(Some(release("v99.0.0"))));
    instance.handle(fetched.unwrap()).unwrap();

    assert_eq!(
        fake.notifications().pop().unwrap().title,
        "New version available: v99.0.0"
    );
}
