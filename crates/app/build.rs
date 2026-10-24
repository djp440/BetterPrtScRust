//! 把应用清单（Per-Monitor V2 DPI 感知等）嵌入 BetterPrtSc.exe。
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
}
