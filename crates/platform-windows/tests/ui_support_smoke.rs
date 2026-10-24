//! 真机冒烟测试：设置界面子进程用到的系统能力（界面语言、系统字体、目录权限、检查更新），默认忽略。
//!
//! 运行：`cargo test -p bps-platform-windows --test ui_support_smoke -- --ignored`
//! 其中 `queries_the_latest_release_on_github` 会向 api.github.com 发一个 GET 请求（ADR 0004
//! 允许的检查更新），需要单独指定才跑。
#![cfg(windows)]

use std::path::PathBuf;

use bps_platform_windows::{
    latest_release, map_system_font, standard_user_can_write, user_ui_languages,
};

#[test]
#[ignore = "真机冒烟：读取当前用户的界面语言"]
fn reports_the_preferred_interface_languages() {
    let languages = user_ui_languages();

    assert!(!languages.is_empty());
    assert!(
        languages.iter().all(|tag| tag.contains('-') || tag.len() == 2),
        "{languages:?}"
    );
}

#[test]
#[ignore = "真机冒烟：只读映射系统字体"]
fn maps_the_interface_fonts_read_only() {
    let segoe = map_system_font("segoeui.ttf").expect("Segoe UI 应当存在");
    assert_eq!(&segoe[..4], &[0, 1, 0, 0], "TrueType 字体文件头");
    if let Some(yahei) = map_system_font("msyh.ttc") {
        assert_eq!(&yahei[..4], b"ttcf", "字体集文件头");
    }
    assert!(map_system_font("no-such-font.ttf").is_none());
}

#[test]
#[ignore = "真机冒烟：检查目录对普通权限的写入权限"]
fn tells_whether_standard_users_can_write_a_folder() {
    assert_eq!(standard_user_can_write(&std::env::temp_dir()), Some(true));
    let windows = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into()));
    assert_eq!(standard_user_can_write(&windows.join("System32")), Some(false));
    assert_eq!(
        standard_user_can_write(&windows.join("System32").join("notepad.exe")),
        Some(false)
    );
}

#[test]
#[ignore = "联网：向 api.github.com 查询一次最新版本（ADR 0004 允许的检查更新）"]
fn queries_the_latest_release_on_github() {
    let release = latest_release("djp440/BetterPrtScRust").expect("查询应当成功");

    // 还没有发布过时为 None；发布过则应有版本号和发布页面。
    if let Some(release) = release {
        assert!(!release.tag.is_empty());
        assert!(release.page_url.starts_with("https://github.com/"));
    }
}
