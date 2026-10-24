## 热键→遮罩上屏延迟（ms，端到端：SendInput → 含哨兵的桌面帧 LastPresentTime）

| 配置 | 首次 中位数 | 首次 P95 | 首次 样本 | 后续 中位数 | 后续 P95 | 后续 样本 | 未上屏 | 旧帧 |
|---|---|---|---|---|---|---|---|---|
| egui-lazy | 578.5 | 656.8 | 4 | - | - | 0 | 12 | 0 |

## 阶段耗时中位数（ms，相邻时间戳之差；input = SendInput→进程收到 WM_HOTKEY）

- egui-lazy 首次：input 4.3 · wake 0.3 · gfx 510.8 · capture 9.5 · upload 11.1 · window 23.3 · present 16.0 · uncloak 0.2 · foreground 26.9 · dwm 1.9 · visible -28.2
- egui-lazy 后续：input 4.0 · wake 0.2 · gfx 465.5 · capture 10.2 · upload 11.0 · window 17.3 · present 13.5 · uncloak 0.3 · foreground 7.7 · dwm 4.3

## 内存（MB，中位数；idle_before = 启动后空闲 2 s，idle_after = 触发完毕、遮罩关闭后空闲 2 s）

| 配置 | 时机 | 工作集 | 私有工作集 | 私有字节 |
|---|---|---|---|---|
| egui-lazy | idle_before | 13.5 | 1.4 | 2.3 |
| egui-lazy | idle_after | 119.3 | 92.3 | 146.5 |

## 启动（ms，中位数）

| 配置 | 进程启动→READY | 其中初始化 |
|---|---|---|
| egui-lazy | 87.8 | 11.5 |

## 拖拽框选验证

- egui-lazy：

## release 二进制体积

| 原型 | exe | zip 压缩后 |
|---|---|---|
| egui | 6.77 MB | 3.13 MB |

## 测量环境

- 时间：2026-10-24 16-16-55；参数：每配置冷启动 4 次 × 每次 1 次首次触发 + 3 次后续触发；遮罩显示 250 ms 后自动关闭；触发间隔随机 250–450 ms；显示顺序模式：默认（Present → 取消 cloak）
- CPU：AMD Ryzen 5 9600X 6-Core Processor；内存：47.2 GB
- 系统：Microsoft Windows 11 专业工作站版 10.0.26300（build 26300）
- 有画面输出的显卡：GameViewer Virtual Display Adapter（驱动 15.6.5.199，3840x2160@144Hz）；NVIDIA GeForce RTX 5070 Ti（驱动 32.0.16.1692，3840x2160@160Hz）；spacedesk Graphics Adapter（驱动 10.2.398.176，1920x1200@60Hz）
- 显示器（DXGI）：\\.\DISPLAY1@0,0,3840x2160,dpi192,rot1,NVIDIA_GeForce_RTX_5070_Ti
- 工具链：rustc 1.95.0 (59807616e 2026-04-14)
