# 选区遮罩延迟原型（ticket #2）

> **一次性原型代码。** 这里的代码只为回答 ADR 0003 的一个问题而写：选区遮罩用 egui（winit + wgpu）
> 能不能做到“按下热键 → 遮罩出现 < 150 ms”，以及它和原生 Win32 + Direct2D 在常驻内存、产物体积上的差距。
> 没有测试、错误处理从简，**不要在正式代码里依赖、复制或“整理后复用”**。正式实现见 #8；
> 结论写在 `docs/adr/0003-egui-ui-with-native-overlay-fallback.md`。

三个 crate 各自独立（每个 `Cargo.toml` 都有空的 `[workspace]`），不属于仓库根 workspace：

| 目录 | 内容 |
|---|---|
| `native-d2d/` | 原生方案：Win32 窗口 + DXGI flip 交换链 + Direct2D；冻结帧在 GPU 上直接包成 D2D 位图 |
| `egui-wgpu/` | egui 方案：winit 0.30 窗口 + wgpu 30（只开 DX12 后端）+ egui-wgpu 0.37；冻结帧读回 CPU 再上传成 wgpu 纹理 |
| `harness/` | 测量工具：SendInput 注入热键、DXGI 观察遮罩上屏时刻、读进程内存、拖拽验证 |
| `common/` | 两个原型共用的截屏（DXGI Desktop Duplication）与计时代码，用 `#[path]` 引入 |
| `run-measure.ps1` | 一键构建并测量，结果写到 `results/<时间戳>/` |

## 运行

```powershell
# 完整测量（约 10 分钟；期间屏幕会反复闪现遮罩，鼠标会被移动一次，请勿操作电脑）
pwsh prototypes/overlay-latency/run-measure.ps1
pwsh prototypes/overlay-latency/run-measure.ps1 -Protos native -Strategies prewarm -ColdRuns 3 -Triggers 5

# 手动试用（多显示器验收用）：按 Ctrl+Alt+Shift+F9 出遮罩，拖拽框选，Esc 或右键关闭；120 s 后进程自动退出
$env:CARGO_TARGET_DIR = "prototypes/overlay-latency/target"
cargo build --release --manifest-path prototypes/overlay-latency/native-d2d/Cargo.toml
prototypes/overlay-latency/target/release/overlay-native.exe --strategy prewarm
```

两个原型参数相同：`--strategy prewarm|ondemand|lazy`、`--hold-ms N`（遮罩显示 N ms 后自动关闭，0 = 只手动关闭）、
`--harness`（从 stdin 读 `hold` / `close` / `quit`，stdin 关闭即退出）、`--exit-after-s N`（最长存活时间）。
遮罩最多显示 8 s 就会被安全阀关掉；热键在进程退出时由系统回收。

| 策略 | 启动时 | 每次触发 |
|---|---|---|
| `prewarm` | 建好 GPU 设备、渲染器、隐藏（cloak）的遮罩窗口与交换链，并真实拍一帧、画两帧（含文字）完整预热 | 拍冻结帧 → 画 → 显示；关闭时只隐藏 |
| `ondemand` | 只建 GPU 设备与渲染器 | 另外创建窗口与交换链，关闭时销毁 |
| `lazy` | 不碰 GPU | 从零创建全部 GPU 资源，关闭后全部释放 |

实验开关（环境变量）：`BPS_SHOW_MODE=flush`（Present 后先 DwmFlush 再取消 cloak；native 另有 `imm` = 首帧 Present(0)）、
`BPS_DUP_DEBUG=1`（stderr 打印每次 AcquireNextFrame 的帧信息）、`BPS_DUP_LEGACY=1`（强制用旧的 `DuplicateOutput`）。

## 测量方法

- **贴近真实架构**：原型是常驻进程，启动后注册全局热键（`RegisterHotKey`，Ctrl+Alt+Shift+F9）；收到 `WM_HOTKEY` 后用
  DXGI Desktop Duplication 拍下每块显示器的冻结帧，再把一个铺满虚拟桌面的置顶窗口显示出来，可拖拽框选。
  进程声明 Per-Monitor V2，所有坐标都是物理像素。
- **注入**：harness 用 `SendInput` 一次性注入 Ctrl↓Alt↓Shift↓F9↓F9↑Shift↑Alt↑Ctrl↑，注入前取 QPC 记为 `t0`。
- **上屏时刻（端到端延迟的终点）**：原型在主显示器 (0,0) 画一个 8×8 的品红哨兵方块，绿色分量编码触发序号；harness 自己开一路
  DXGI Desktop Duplication 盯着这块像素，哨兵第一次出现在桌面帧里时取该帧的 `LastPresentTime`（QPC，DWM 把这一帧呈现到屏幕的时刻）。
  端到端延迟 = `LastPresentTime − t0`。这不依赖原型自报，也能发现“先闪出上一次的旧画面”（序号不符的哨兵帧，表里的“旧帧”列）。
- **阶段拆分**：原型在 stdout 报告各阶段 QPC（收到热键、拍完、上传完、Present 返回、取消 cloak、抢到前台、DwmFlush 返回），
  harness 换算成相邻差值。
- **首次 / 后续**：每个配置冷启动进程 8 次；每次启动后空闲 2 s，第 1 次触发记“冷启动后首次”，接着 12 次记“后续”。
  遮罩显示 250 ms 后自动关闭，两次触发间隔随机 250–450 ms（避免与刷新周期锁相）。
- **内存**：`GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS_EX2)`，启动后空闲 2 s 记一次（idle_before），全部触发结束、
  遮罩关闭后空闲 2 s 再记一次（idle_after）。“私有工作集”就是任务管理器“内存”列显示的值；“私有字节”是提交量，含显卡驱动的大块保留。
  显存（GPU 专用内存）不在其中。
- **体积**：release 配置 `opt-level=3, lto=true, codegen-units=1, strip=true, panic="abort"`，另报 zip 压缩后的大小。
- **拖拽验证**：每个配置额外冷启动一次，注入一次对角拖拽（主显示器 30%→60%），核对原型报告的选区矩形，截下遮罩画面
  （`results/**/*.png`，含本机桌面内容，不入库），并比较选区内像素与冻结前桌面的平均色差。注入鼠标前确认光标下是原型窗口，
  注入 Esc 前确认原型在前台，结束后把光标放回原处。

## 结果

2026-10-24 在本机（Ryzen 5 9600X、RTX 5070 Ti、单块 4K@160 Hz、缩放 200%、Windows 11 build 26300）跑了四次：

| 目录 | 内容 |
|---|---|
| `results/2026-10-24 16-02-40/` | 两个原型 × `prewarm`、`ondemand`，8 次冷启动 × (1 + 12) 次触发，含拖拽验证 |
| `results/2026-10-24 16-10-56 flush/` | 两个原型 × `prewarm`，`BPS_SHOW_MODE=flush` |
| `results/2026-10-24 16-14-39/` | 原生 × `lazy` |
| `results/2026-10-24 16-16-55/` | egui × `lazy`（4 次冷启动 × (1 + 3) 次，第二次起都没上屏） |

每个目录里有 `summary.md`（汇总表）、`triggers.csv`（每次触发的端到端延迟与阶段耗时）、`memory.csv`、
`harness.log`（原型 stdout 与观察到的帧轨迹）。

| 方案 · 策略 | 首次 中位数 / P95 (ms) | 后续 中位数 / P95 (ms) | 空闲私有工作集 (MB) | exe |
|---|---|---|---|---|
| 原生 · prewarm | 24.7 / 33.9 | 21.9 / 30.6 | 14.4 | 0.21 MB |
| egui · prewarm | 38.6 / 51.3 | 40.0 / 52.1 | 93.7 | 6.77 MB |

结论：egui 延迟达标，但内存超标，选区遮罩改用原生绘制。完整数据见 ADR 0003，决策见 ADR 0005。

## 待手动验收（本机只有一块显示器）

多显示器、混合缩放：在至少两块缩放比例不同（如 100% + 150%）、最好有一块在主显示器左侧或上方（负坐标）的环境里，
分别运行两个原型（`--strategy prewarm`），按热键后确认：

1. 遮罩覆盖所有显示器，每块显示器上的冻结帧与原画面对齐、不模糊、没有错位或缩放；
2. 从一块显示器拖到另一块时选区连续，stdout 打印的 `SELECTION` 是虚拟桌面物理像素坐标；
3. 尺寸标签和边框粗细按选区左上角所在显示器的缩放变化；
4. 遮罩出现时窗口没有被系统按 DPI 改变位置或大小（两个原型都吞掉了 `WM_DPICHANGED`）。

原型没有处理旋转的显示器（Desktop Duplication 返回的是未旋转的图像），竖屏会显示错误。
