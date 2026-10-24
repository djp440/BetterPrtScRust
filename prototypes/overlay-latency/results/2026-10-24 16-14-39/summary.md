## 热键→遮罩上屏延迟（ms，端到端：SendInput → 含哨兵的桌面帧 LastPresentTime）

| 配置 | 首次 中位数 | 首次 P95 | 首次 样本 | 后续 中位数 | 后续 P95 | 后续 样本 | 未上屏 | 旧帧 |
|---|---|---|---|---|---|---|---|---|
| native-lazy | 168.8 | 197.1 | 8 | 143.2 | 260.6 | 96 | 0 | 0 |

## 阶段耗时中位数（ms，相邻时间戳之差；input = SendInput→进程收到 WM_HOTKEY）

- native-lazy 首次：input 3.8 · dispatch 0.0 · gfx 132.4 · capture 3.7 · bitmap 0.0 · window 14.9 · present 7.4 · uncloak 0.2 · foreground 35.1 · dwm 4.0 · visible -35.4
- native-lazy 后续：input 4.4 · dispatch 0.0 · gfx 101.0 · capture 4.2 · bitmap 0.0 · window 14.7 · present 8.1 · uncloak 0.3 · foreground 24.5 · dwm 4.5 · visible -22.6

## 内存（MB，中位数；idle_before = 启动后空闲 2 s，idle_after = 触发完毕、遮罩关闭后空闲 2 s）

| 配置 | 时机 | 工作集 | 私有工作集 | 私有字节 |
|---|---|---|---|---|
| native-lazy | idle_before | 11.5 | 1.2 | 2.0 |
| native-lazy | idle_after | 43.2 | 12.6 | 123.7 |

## 启动（ms，中位数）

| 配置 | 进程启动→READY | 其中初始化 |
|---|---|---|
| native-lazy | 55.2 | 0.0 |

## 拖拽框选验证

- native-lazy：选区 1152,648 1152×540（期望 1152,648 1152×540）一致；截图 native-lazy-drag.png；选区内与原桌面平均色差 0.00；Esc 关闭：esc

## release 二进制体积

| 原型 | exe | zip 压缩后 |
|---|---|---|
| native | 0.21 MB | 0.11 MB |

## 测量环境

- 时间：2026-10-24 16-14-39；参数：每配置冷启动 8 次 × 每次 1 次首次触发 + 12 次后续触发；遮罩显示 250 ms 后自动关闭；触发间隔随机 250–450 ms；显示顺序模式：默认（Present → 取消 cloak）
- CPU：AMD Ryzen 5 9600X 6-Core Processor；内存：47.2 GB
- 系统：Microsoft Windows 11 专业工作站版 10.0.26300（build 26300）
- 有画面输出的显卡：GameViewer Virtual Display Adapter（驱动 15.6.5.199，3840x2160@144Hz）；NVIDIA GeForce RTX 5070 Ti（驱动 32.0.16.1692，3840x2160@160Hz）；spacedesk Graphics Adapter（驱动 10.2.398.176，1920x1200@60Hz）
- 显示器（DXGI）：\\.\DISPLAY1@0,0,3840x2160,dpi192,rot1,NVIDIA_GeForce_RTX_5070_Ti
- 工具链：rustc 1.95.0 (59807616e 2026-04-14)
