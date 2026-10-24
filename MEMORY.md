# 开发记忆

- 2026-10-24 15:55 DXGI Desktop Duplication：新建 Duplication 后第一次 `AcquireNextFrame` 常返回 `LastPresentTime == 0` 的全黑帧，必须跳过、等下一帧（静止桌面也会很快补一帧）。HDR / 高级颜色桌面上即使 `DuplicateOutput1` 只声明 BGRA8，仍会给出 `R16G16B16A16_FLOAT`（线性 scRGB，1.0 = 80 尼特），要按 DisplayConfig 的 SDR 白电平归一化后再 sRGB 编码，见 `platform-windows/src/pixels.rs`。
- 2026-10-24 15:55 自动化验证热键时不要用 SendInput 模拟单独的 Shift/Alt：松开时可能切换输入法中英文或激活前台程序菜单栏；冒烟测试用 Ctrl+F24，端到端脚本直接向隐藏窗口投递 `WM_HOTKEY`。
- 2026-10-24 15:55 PowerShell 脚本：Windows PowerShell 5.1 读无 BOM 的 UTF-8 脚本会乱码，含中文的 `.ps1` 要带 BOM；向 .NET/P/Invoke 的 `string` 参数传 `$null` 会变成空字符串，需要 null 时用 `[NullString]::Value`；访问剪贴板要 `-STA`。
