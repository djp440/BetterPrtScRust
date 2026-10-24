# Windows 先行，平台抽象按能力划分

首发只支持 Windows（最低 Win10 1903），但从一开始就使用 Cargo workspace：`core` 存放平台无关的逻辑，`platform` 只定义 trait，`platform-windows` 是当前唯一的实现。trait 按能力划分（截屏、热键、窗口枚举、OCR、托盘、开机自启），不按 Win32 API 划分，这样以后接入 macOS（ScreenCaptureKit、Vision）和 Linux（X11 / Wayland portal）时不需要改动 `core`。

## Consequences

- 截图（显示器、区域、虚拟桌面）走 DXGI Desktop Duplication，速度快、没有边框；录屏和窗口模式走 Windows.Graphics.Capture，能截到被遮挡的窗口。
- OCR 使用平台自带的引擎（Windows.Media.Ocr），所以识别质量会随平台而不同；如果以后需要各平台一致的识别结果，再提供可选下载的统一引擎。
