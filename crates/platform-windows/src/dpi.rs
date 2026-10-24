//! Per-Monitor V2 DPI 感知。
//!
//! 常驻实例通过可执行文件内嵌的清单（`crates/app/BetterPrtSc.manifest`）声明；
//! 没有清单的进程（如测试）调用 [`ensure_per_monitor_v2`] 在运行时声明。

use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
};

/// 当前线程（默认继承进程设置）是否为 Per-Monitor V2 DPI 感知。
pub fn is_per_monitor_v2() -> bool {
    unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
        .as_bool()
    }
}

/// 确保进程是 Per-Monitor V2 DPI 感知：已经是（例如清单已声明）时什么都不做，
/// 否则在运行时声明。必须在创建任何窗口之前调用。返回最终是否为 Per-Monitor V2。
pub fn ensure_per_monitor_v2() -> bool {
    if is_per_monitor_v2() {
        return true;
    }
    // 进程的 DPI 感知只能设置一次；失败时以实际状态为准。
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    is_per_monitor_v2()
}
