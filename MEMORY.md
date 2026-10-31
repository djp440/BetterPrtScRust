# 开发记忆

- 2026-10-31 20:59 #33 / #34：用户确认人工验收通过，四项体验清单已勾选。release 版调整选区的视觉流畅度明显优于 debug；这是用户对比反馈，尚未量化各环节耗时，不据此断定唯一性能根因。正式验收及后续复测使用 target/release/BetterPrtSc.exe，切换版本前退出旧实例。

- 2026-10-31 20:43 #33 / #34：界面快捷键配置为 ocr_shortcut / translate_shortcut，旧配置默认键与全局键冲突时只禁用冲突默认键；普通区域撤销历史只存范围、从同一冻结帧重裁，窗口来源保留共享原图和身份。已有常驻 exe 占用 target/debug 时，用独立 target/agent-validation 构建验证；切换验收版本须先退出旧托盘实例，否则会转交给旧进程。人工体验清单见 docs/checklist/33-34-界面快捷键与标注后调整.md。

- 2026-10-31 19:33 AV1 / Windows 10 MP4：NVIDIA MFT 与 sink 已有序列头仍可在 Finalize 报 `0xC00D4A45`；需按自定义格式约定设置完整 stsd / av01 / av1C。av01 必须用序列最大尺寸，实际 render 尺寸不同时按 AV1 ISOBMFF 添加 pasp，不能写成实际小帧尺寸。系统解码测试还需 D3D 管理器和高级视频处理，保留原像素比例，处理 CURRENTMEDIATYPECHANGED，并优先用 IMF2DBuffer 的真实 pitch；基础软件转换的 `0xC00D36B1` 不能直接归因于视频或显卡驱动。对齐缓冲区尺寸也不等于显示尺寸（如 H.264 34×34 返回 48×48），读取后需裁去填充。小尺寸至 4K 已通过 Windows 和 FFmpeg 解码；独立元数据解析和正负行跨度测试进入默认回归，见 `docs/diagnostics/31-av1-mp4.md`。
- 2026-10-24 16:43 冒烟脚本启动 BetterPrtSc.exe / bps.exe 前要把子进程的 `APPDATA` 指向临时目录（或用便携模式），否则常驻实例首次运行会在真实的 `%APPDATA%\BetterPrtSc` 写入 config.toml；`finally` 里恢复剪贴板可能因剪贴板被占用而抛异常，要 try/catch，否则会跳过后面删除截图等清理。
- 2026-10-24 16:42 发布脚本（`scripts/release/`）：本机只有 Pester 3.4、CI 上是 5.x，断言语法互不兼容，所以 `test-release.ps1` 自带极简断言、不依赖 Pester。常驻内存判定看私有工作集（ADR 0005），用 `K32GetProcessMemoryInfo` + `PROCESS_MEMORY_COUNTERS_EX2` 读取（`Get-Process` 没有这个值，性能计数器名称又是本地化的）。
- 2026-10-24 16:23 PowerShell 把中文弯引号 `“` `”` 也当作双引号：双引号字符串里写 `“$x”` 会直接解析失败，含弯引号的文本放进单引号字符串或改用「」等其他标点。
- 2026-10-24 16:23 配置文件监视用 `FindFirstChangeNotificationW` 监视所在目录，句柄由事件循环的 `MsgWaitForMultipleObjectsEx` 一并等待（不另开线程），目录变化后 300 ms 定时器合并编辑器的多次写入；目录变化不一定是 config.toml 本身，`core` 重新解析后与当前配置相等就什么都不做。句柄打开期间删不掉该目录。
- 2026-10-24 16:16 单实例与 CLI：命名管道 `\\.\pipe\BetterPrtSc-<会话号>` 既是单实例锁（`FILE_FLAG_FIRST_PIPE_INSTANCE`，失败即已有实例）也是转交通道；监听线程必须先建好下一个管道实例再处理当前连接，否则管道短暂消失会让第二个进程当上常驻实例。`bps` 拉起常驻实例时标准输入输出必须设为 null（否则调用 `bps` 的脚本会一直等继承的输出管道关闭），并用 `DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB`（作业不允许时退回不脱离），工作目录设为程序目录。冒烟脚本只认被测目录里的进程，别按进程名或窗口类名误关其他工作树的实例。
- 2026-10-24 15:55 DXGI Desktop Duplication：新建 Duplication 后第一次 `AcquireNextFrame` 常返回 `LastPresentTime == 0` 的全黑帧，必须跳过、等下一帧（静止桌面也会很快补一帧）。HDR / 高级颜色桌面上即使 `DuplicateOutput1` 只声明 BGRA8，仍会给出 `R16G16B16A16_FLOAT`（线性 scRGB，1.0 = 80 尼特），要按 DisplayConfig 的 SDR 白电平归一化后再 sRGB 编码，见 `platform-windows/src/pixels.rs`。
- 2026-10-24 15:55 自动化验证热键时不要用 SendInput 模拟单独的 Shift/Alt：松开时可能切换输入法中英文或激活前台程序菜单栏；冒烟测试用 Ctrl+F24，端到端脚本直接向隐藏窗口投递 `WM_HOTKEY`。
- 2026-10-24 15:55 PowerShell 脚本：Windows PowerShell 5.1 读无 BOM 的 UTF-8 脚本会乱码，含中文的 `.ps1` 要带 BOM；向 .NET/P/Invoke 的 `string` 参数传 `$null` 会变成空字符串，需要 null 时用 `[NullString]::Value`；访问剪贴板要 `-STA`。
