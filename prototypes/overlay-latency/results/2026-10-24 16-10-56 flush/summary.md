## 热键→遮罩上屏延迟（ms，端到端：SendInput → 含哨兵的桌面帧 LastPresentTime）

| 配置 | 首次 中位数 | 首次 P95 | 首次 样本 | 后续 中位数 | 后续 P95 | 后续 样本 | 未上屏 | 旧帧 |
|---|---|---|---|---|---|---|---|---|
| native-prewarm | 29.7 | 42.1 | 8 | 28.6 | 59.5 | 96 | 0 | 0 |
| egui-prewarm | 50.5 | 66.6 | 8 | 52.4 | 71.8 | 96 | 0 | 0 |

## 阶段耗时中位数（ms，相邻时间戳之差；input = SendInput→进程收到 WM_HOTKEY）

- native-prewarm 首次：input 4.0 · dispatch 0.0 · capture 4.1 · bitmap 0.0 · present 7.1 · preflush 4.5 · uncloak 0.3 · foreground 35.2 · dwm 3.0 · visible -35.7
- native-prewarm 后续：input 4.4 · dispatch 0.0 · capture 4.8 · bitmap 0.0 · present 7.7 · preflush 5.1 · uncloak 0.4 · foreground 11.1 · dwm 6.2 · visible -12.1
- egui-prewarm 首次：input 3.8 · wake 0.3 · capture 9.7 · upload 11.0 · present 8.8 · preflush 8.4 · uncloak 0.3 · foreground 19.0 · dwm 4.1 · visible -17.0
- egui-prewarm 后续：input 4.7 · wake 0.2 · capture 9.4 · upload 11.5 · present 7.9 · preflush 10.8 · uncloak 0.4 · foreground 8.9 · dwm 3.8 · visible -6.0

## 内存（MB，中位数；idle_before = 启动后空闲 2 s，idle_after = 触发完毕、遮罩关闭后空闲 2 s）

| 配置 | 时机 | 工作集 | 私有工作集 | 私有字节 |
|---|---|---|---|---|
| native-prewarm | idle_before | 50.7 | 14.3 | 121.3 |
| native-prewarm | idle_after | 59.4 | 16.1 | 123.5 |
| egui-prewarm | idle_before | 139.3 | 93.8 | 328.0 |
| egui-prewarm | idle_after | 134.5 | 88.0 | 319.1 |

## 启动（ms，中位数）

| 配置 | 进程启动→READY | 其中初始化 |
|---|---|---|
| native-prewarm | 231.2 | 199.8 |
| egui-prewarm | 626.5 | 584.0 |

## 拖拽框选验证

- native-prewarm：
- egui-prewarm：

## release 二进制体积

| 原型 | exe | zip 压缩后 |
|---|---|---|
| native | 0.21 MB | 0.11 MB |
| egui | 6.77 MB | 3.13 MB |

## 测量环境

- 时间：2026-10-24 16-10-56；参数：每配置冷启动 8 次 × 每次 1 次首次触发 + 12 次后续触发；遮罩显示 250 ms 后自动关闭；触发间隔随机 250–450 ms；显示顺序模式：flush
- CPU：AMD Ryzen 5 9600X 6-Core Processor；内存：47.2 GB
- 系统：Microsoft Windows 11 专业工作站版 10.0.26300（build 26300）
- 有画面输出的显卡：GameViewer Virtual Display Adapter（驱动 15.6.5.199，3840x2160@144Hz）；NVIDIA GeForce RTX 5070 Ti（驱动 32.0.16.1692，3840x2160@160Hz）；spacedesk Graphics Adapter（驱动 10.2.398.176，1920x1200@60Hz）
- 显示器（DXGI）：\\.\DISPLAY1@0,0,3840x2160,dpi192,rot1,NVIDIA_GeForce_RTX_5070_Ti
- 工具链：rustc 1.95.0 (59807616e 2026-04-14)
