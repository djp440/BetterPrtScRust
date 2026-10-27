//! 把应用清单（Per-Monitor V2 DPI 感知等）嵌入 BetterPrtSc.exe，并让常驻进程用不到的 DLL
//! 延迟加载。
//!
//! 直接使用 MSVC 链接器的 `/MANIFEST:EMBED` + `/MANIFESTINPUT`，不引入额外构建依赖。

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=BetterPrtSc.manifest");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" {
        return;
    }
    if target_env != "msvc" {
        println!("cargo:warning=非 MSVC 工具链不会嵌入应用清单，程序将在运行时声明 DPI 感知");
        return;
    }
    let manifest =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("BetterPrtSc.manifest");
    println!("cargo:rustc-link-arg-bin=BetterPrtSc=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=BetterPrtSc=/MANIFESTINPUT:{}",
        manifest.display()
    );
    // 常驻进程用不到的 DLL 改为延迟加载（第一次调用时才载入）：OpenGL 只有设置界面等窗口
    // 子进程用；WinHTTP 只有开启“检查更新”后才用；Media Foundation 和 DirectComposition 只有
    // 录屏时才用。
    for dll in DELAY_LOADED {
        println!("cargo:rustc-link-arg-bin=BetterPrtSc=/DELAYLOAD:{dll}");
    }
    println!("cargo:rustc-link-arg-bin=BetterPrtSc=delayimp.lib");
}

const DELAY_LOADED: [&str; 5] = [
    "opengl32.dll",
    "winhttp.dll",
    "mfplat.dll",
    "mfreadwrite.dll",
    "dcomp.dll",
];
