## 热键→遮罩上屏延迟（ms，端到端：SendInput → 含哨兵的桌面帧 LastPresentTime）

| 配置 | 首次 中位数 | 首次 P95 | 首次 样本 | 后续 中位数 | 后续 P95 | 后续 样本 | 未上屏 | 旧帧 |
|---|---|---|---|---|---|---|---|---|
| native-prewarm | 24.7 | 33.9 | 8 | 21.9 | 30.6 | 96 | 0 | 2 |
| native-ondemand | 37.4 | 45.6 | 8 | 36.7 | 49.0 | 96 | 0 | 0 |
| egui-prewarm | 38.6 | 51.3 | 8 | 40.0 | 52.1 | 96 | 0 | 3 |
| egui-ondemand | 66.0 | 74.6 | 8 | 56.1 | 73.4 | 96 | 0 | 0 |

## 阶段耗时中位数（ms，相邻时间戳之差；input = SendInput→进程收到 WM_HOTKEY）

- native-prewarm 首次：input 4.3 · dispatch 0.0 · capture 4.6 · bitmap 0.0 · present 6.8 · uncloak 0.3 · foreground 50.7 · dwm 5.9 · visible -53.6
- native-prewarm 后续：input 4.2 · dispatch 0.0 · capture 4.6 · bitmap 0.0 · present 7.1 · uncloak 0.4 · foreground 23.8 · dwm 4.8 · visible -24.1
- native-ondemand 首次：input 3.4 · dispatch 0.0 · capture 3.7 · bitmap 0.0 · window 15.4 · present 7.7 · uncloak 0.2 · foreground 34.8 · dwm 6.5 · visible -36.0
- native-ondemand 后续：input 4.0 · dispatch 0.0 · capture 4.9 · bitmap 0.0 · window 12.2 · present 6.3 · uncloak 0.3 · foreground 36.1 · dwm 3.6 · visible -33.9
- egui-prewarm 首次：input 3.9 · wake 0.2 · capture 8.0 · upload 10.6 · present 8.1 · uncloak 0.2 · foreground 24.8 · dwm 5.9 · visible -41.0
- egui-prewarm 后续：input 4.4 · wake 0.2 · capture 8.8 · upload 11.3 · present 7.9 · uncloak 0.3 · foreground 19.3 · dwm 4.8 · visible -18.1
- egui-ondemand 首次：input 3.7 · wake 0.2 · capture 10.1 · upload 11.1 · window 21.5 · present 13.1 · uncloak 0.3 · foreground 26.4 · dwm 6.5 · visible -24.3
- egui-ondemand 后续：input 4.2 · wake 0.2 · capture 8.9 · upload 11.1 · window 17.8 · present 6.4 · uncloak 0.1 · foreground 19.1 · dwm 4.5 · visible -16.9

## 内存（MB，中位数；idle_before = 启动后空闲 2 s，idle_after = 触发完毕、遮罩关闭后空闲 2 s）

| 配置 | 时机 | 工作集 | 私有工作集 | 私有字节 |
|---|---|---|---|---|
| native-prewarm | idle_before | 50.8 | 14.4 | 121.3 |
| native-prewarm | idle_after | 59.3 | 16.0 | 123.4 |
| native-ondemand | idle_before | 39.4 | 10.1 | 52.7 |
| native-ondemand | idle_after | 57.6 | 15.3 | 122.8 |
| egui-prewarm | idle_before | 139.1 | 93.7 | 328.5 |
| egui-prewarm | idle_after | 134.8 | 88.2 | 320.9 |
| egui-ondemand | idle_before | 97.9 | 58.7 | 200.5 |
| egui-ondemand | idle_after | 133.7 | 88.1 | 319.2 |

## 启动（ms，中位数）

| 配置 | 进程启动→READY | 其中初始化 |
|---|---|---|
| native-prewarm | 209.3 | 179.8 |
| native-ondemand | 177.2 | 150.0 |
| egui-prewarm | 626.5 | 579.9 |
| egui-ondemand | 565.4 | 519.9 |

## 拖拽框选验证

- native-prewarm：选区 1152,648 1152×540（期望 1152,648 1152×540）一致；截图 native-prewarm-drag.png；选区内与原桌面平均色差 0.00；Esc 关闭：esc
- native-ondemand：选区 1152,648 1152×540（期望 1152,648 1152×540）一致；截图 native-ondemand-drag.png；选区内与原桌面平均色差 0.00；Esc 关闭：esc
- egui-prewarm：选区 1152,648 1152×540（期望 1152,648 1152×540）一致；截图 egui-prewarm-drag.png；选区内与原桌面平均色差 0.00；Esc 关闭：esc
- egui-ondemand：选区 1152,648 1152×540（期望 1152,648 1152×540）一致；截图 egui-ondemand-drag.png；选区内与原桌面平均色差 0.00；Esc 关闭：esc

## release 二进制体积

| 原型 | exe | zip 压缩后 |
|---|---|---|
| native | 0.21 MB | 0.11 MB |
| egui | 6.77 MB | 3.13 MB |

## 测量环境

- 时间：2026-10-24 16-02-40；参数：每配置冷启动 8 次 × 每次 1 次首次触发 + 12 次后续触发；遮罩显示 250 ms 后自动关闭；触发间隔随机 250–450 ms；显示顺序模式：默认（Present → 取消 cloak）
- CPU：AMD Ryzen 5 9600X 6-Core Processor；内存：47.2 GB
- 系统：Microsoft Windows 11 专业工作站版 10.0.26300（build 26300）
- 有画面输出的显卡：GameViewer Virtual Display Adapter（驱动 15.6.5.199，3840x2160@144Hz）；NVIDIA GeForce RTX 5070 Ti（驱动 32.0.16.1692，3840x2160@160Hz）；spacedesk Graphics Adapter（驱动 10.2.398.176，1920x1200@60Hz）
- 显示器（DXGI）：\\.\DISPLAY1@0,0,3840x2160,dpi192,rot1,NVIDIA_GeForce_RTX_5070_Ti
- 工具链：rustc 1.95.0 (59807616e 2026-04-14)
