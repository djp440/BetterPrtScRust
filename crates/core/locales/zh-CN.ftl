# BetterPrtSc 界面文本：简体中文。
# 新增文本时同时在 en.ftl 里加上同名消息（有测试检查两份资源的消息是否一一对应）。

## 托盘菜单

tray-settings = 设置
tray-exit = 退出

## 通知

notification-hotkey-failed-title = 热键注册失败
notification-hotkey-failed-line = { $hotkey }：{ $reason }
notification-capture-failed-title = 捕获失败
notification-saved-title = 截图已保存
notification-save-failed-title = 自动存储失败
notification-recording-saved-title = 录屏已保存
notification-recording-in-progress-title = 正在录屏
notification-recording-no-audio-title = 录屏没有声音
notification-recording-no-audio-body = 无法录制系统声音，这段录屏照常录制画面：{ $reason }
notification-recording-audio-lost-title = 录屏的声音中断了
notification-recording-audio-lost-body = 之后的部分没有声音，画面照常录制：{ $reason }
notification-recording-no-microphone-title = 录屏没有麦克风声音
notification-recording-no-microphone-body = 无法录制麦克风，这段录屏照常录制画面和系统声音：{ $reason }
notification-recording-microphone-fallback-title = 录屏改用默认麦克风
notification-recording-microphone-fallback-body = 选中的麦克风不可用，这段录屏改用系统默认麦克风：{ $reason }
notification-recording-microphone-lost-title = 录屏的麦克风中断了
notification-recording-microphone-lost-body = 之后的部分没有麦克风声音，画面照常录制：{ $reason }
notification-recording-codec-fallback-title = 录屏改用 H.264
notification-recording-codec-fallback-body = 无法用 { $codec } 编码，这段录屏改用 H.264：{ $reason }
notification-config-unwatchable-title = 无法监视配置文件
notification-config-unwatchable-body = 修改配置文件后需要重启 BetterPrtSc 才能生效。{ $error }
notification-config-unwritable-title = 无法写入配置文件
notification-config-unwritable-body = 本次使用默认配置。{ $error }
notification-config-invalid-title = 配置文件有错误
notification-config-unreadable-title = 无法读取配置文件
notification-config-startup-fallback = 本次暂时使用默认配置。{ $error }
notification-config-reload-fallback = 继续使用上一次有效的配置。{ $error }
notification-config-deleted-title = 配置文件已被删除
notification-config-deleted-body = 继续使用当前的配置；重启 BetterPrtSc 后会重新生成默认配置。
notification-settings-unavailable-title = 无法打开设置界面
notification-result-panel-unavailable-title = 无法打开结果面板
notification-ocr-copied-title = 已复制 { $count } 个字
notification-ocr-copied-body = OCR 识别出的文字已在剪贴板里。
notification-ocr-empty-title = 没有识别到文字
notification-ocr-empty-body = 剪贴板保持不变。
notification-ocr-no-language-pack-title = 系统没有可用的 OCR 语言包
notification-ocr-no-language-pack-body = 剪贴板保持不变。点击打开系统的“语言和区域”设置，添加要识别的语言即可安装 OCR 组件。
notification-ocr-failed-unknown = 原因不明，请改用结果面板重试。
notification-ocr-failed-title = OCR 识别失败
notification-update-available-title = 发现新版本 { $version }
notification-update-available-body = 当前版本 { $current }。点击打开发布页面，自行下载。
update-unrecognized-version = 无法识别 GitHub Releases 上的版本号“{ $version }”
notification-prtsc-taken-title = PrtSc 键被系统截图工具占用
notification-prtsc-taken-body = 点击打开系统设置，关闭“使用 Print Screen 键打开屏幕截图”后，BetterPrtSc 会立即接管 PrtSc 键，无需重启。
notification-prtsc-available-title = PrtSc 键已可用
notification-prtsc-available-body = 系统截图工具不再占用 PrtSc 键，相应的热键绑定已生效。
notification-not-elevated-title = 未能以管理员身份运行
notification-not-elevated-body = { $reason }。本次以普通权限运行：管理员权限的窗口在前台时，热键不会生效。
not-elevated-after-restart = 以管理员身份重新启动后仍然没有管理员权限
notification-autostart-unreadable-title = 无法读取开机自启状态
notification-autostart-enabled-portable-title = 已开启开机自启
notification-autostart-enabled-portable-body = 便携模式下，开机自启指向当前位置的程序：移动程序目录后需要重新开启（从新位置启动一次 BetterPrtSc 即可）。
notification-autostart-enable-failed-title = 无法开启开机自启
notification-autostart-disable-failed-title = 无法关闭开机自启

## 捕获模式与截后动作

mode-display = 显示器
mode-virtual-desktop = 虚拟桌面
mode-region = 区域
action-copy-to-clipboard = 复制到剪贴板
action-await-confirmation = 等待确认
action-record = 录屏
action-long-screenshot = 长截屏
action-ocr = OCR

## 执行失败的原因

error-platform = 平台调用失败：{ $error }
error-no-display = 没有可用的显示器
error-encode = 截图编码失败：{ $error }
error-too-many-same-name-files = 同名文件过多，找不到可用的文件名：{ $path }
error-invalid-request = 常驻实例无法识别请求“{ $request }”，请确认 bps 与 BetterPrtSc 的版本一致
error-capture-in-progress = 上一次捕获还没有结束
error-recording-region-too-small = 区域 { $width } × { $height } 太小，无法录屏（宽和高至少 { $minimum } 像素）
error-nothing-recorded = 没有录到任何画面
error-recording-in-progress-hotkey = 正在录屏，按 { $hotkey } 停止录制后再捕获
error-recording-in-progress = 正在录屏，停止录制后再捕获

## 热键注册失败的原因

registration-duplicate = 与第 { $index } 条热键绑定的热键重复
registration-taken = 已被其他程序占用
registration-taken-by-system-screenshot-tool = 已被系统截图工具占用（可在系统设置里关闭）

## 配置文件与设置的错误

config-error-at-line = 第 { $line } 行：{ $message }
config-hotkey-not-a-modifier = 热键“{ $hotkey }”中的“{ $part }”不是修饰键（Ctrl、Alt、Shift、Win）
config-hotkey-missing-key = 热键“{ $hotkey }”缺少主键
config-hotkey-unsupported-key = 热键“{ $hotkey }”中的“{ $key }”不是支持的按键
config-hotkey-typing-key = 热键“{ $hotkey }”会吞掉正常输入：字母和数字键需要搭配 Ctrl、Alt 或 Win
config-unknown-format = 未知的输出格式“{ $format }”：截图可用 png、jpg、webp、avif，录屏可用 mp4、gif
config-image-format-for-recording = 录屏的 output_format 只能是 mp4 或 gif
config-legacy-actions = 旧的 actions = [...] 写法已不再支持：截图类绑定改写 after = "copy_to_clipboard" 或 "await_confirmation"（自动存储改为 auto_save = true），直达功能改写 function = "record"、"long_screenshot" 或 "ocr"
config-unknown-mode = 未知的捕获模式“{ $mode }”：可用 display（显示器）、virtual_desktop（虚拟桌面）、region（区域）
config-window-mode-merged = 窗口模式已并入区域模式：把 mode = "window" 改成 mode = "region"，在遮罩上单击窗口即可截取它，拖拽则框选区域
config-unknown-after = 未知的截后动作“{ $after }”，可选 copy_to_clipboard、await_confirmation
config-unknown-function = 未知的直达功能“{ $function }”，可选 record、long_screenshot、ocr
config-after-with-function = after 与 function 只能写一个：截图类绑定写 after，直达功能写 function
config-auto-save-not-applicable = 直达功能 { $function } 不支持 auto_save（录屏总是保存，OCR 不保存）
config-recording-format-for-screenshot = mp4、gif 只用于录屏（function = "record" 的绑定）；截图可用 png、jpg、webp、avif
config-recording-format-in-global = 全局的 output_format 只用于截图，可用 png、jpg、webp、avif；录屏格式（mp4、gif）写在 function = "record" 的绑定里
config-zero-frame-rate = frame_rate 必须大于 0
config-unknown-video-codec = 未知的视频编码“{ $codec }”：可用 h264、h265、av1
config-unknown-video-quality = 未知的画质“{ $quality }”：可用 low、standard、high；自定义码率写 video_bitrate
config-zero-bitrate = video_bitrate 必须大于 0
config-unknown-language = 未知的界面语言“{ $language }”：可用 auto（跟随系统）、zh-CN、en
template-unclosed = 命名模板中的 { "{" } 没有对应的 { "}" }
template-unmatched-closing = 命名模板中有单独的 { "}" }，字面的花括号请写成 { "{{" } 或 { "}}" }
template-unknown-placeholder = 命名模板中有无法识别的占位符 { "{" }{ $name }{ "}" }；可用 { "{" }mode{ "}" }、{ "{" }window_title{ "}" } 或由 yyyy、yy、MM、M、dd、d、HH、H、mm、m、ss、s、fff 组成的日期时间格式

## 设置界面：保存前检查出的问题

settings-problem-empty-save-dir = 自动存储目录不能为空；不想指定时请选“默认目录”
settings-problem-typing-key = 第 { $index } 条热键绑定（{ $hotkey }）：字母和数字键需要搭配 Ctrl、Alt 或 Win
settings-problem-missing-hotkey = 第 { $index } 条热键绑定没有设置热键
settings-problem-duplicate-hotkey = 第 { $index } 条热键绑定（{ $hotkey }）与第 { $other } 条重复，请换一个组合键
settings-group-general = 常规
settings-group-hotkeys = 热键
settings-group-saving = 截图与保存
settings-group-recording = 录屏
settings-group-about = 关于
settings-about-license = 许可证：{ $license }
settings-about-project = 项目地址
settings-hotkey-unset = 未设置
settings-hotkey-record-tooltip = 点击后按下新的组合键
settings-record-prompt = 请按下组合键…
settings-record-help = Esc 取消，Backspace / Delete 清空
settings-record-unsupported = 不支持这个键：热键可用 PrintScreen、F1–F24、A–Z、0–9
settings-record-unavailable = 无法录制热键：{ $error }
settings-problem-invalid = 无法生成有效的配置文件：{ $error }

## 设置界面

settings-window-title = BetterPrtSc 设置
settings-resident-not-running = BetterPrtSc 没有在运行：保存的修改会在下次启动时生效，热键注册情况也要到那时才知道。
settings-section-general = 常规
settings-language = 界面语言
settings-language-auto = 跟随系统
settings-autostart = 开机时自动启动
settings-run-as-admin = 以管理员身份运行
settings-run-as-admin-hint = 以管理员身份运行的窗口在前台时，热键也能生效。开启时会请求一次管理员权限。
settings-run-as-admin-risk = 注意：程序所在的文件夹可以被普通权限的程序改写，它们可借此在你下次登录时获得管理员权限。建议改为“为所有用户安装”（安装到 Program Files）后再开启。
settings-check-for-updates = 检查更新
settings-check-for-updates-hint = 只向 GitHub Releases 查询最新的版本号，有新版本时提示，不下载任何东西。关闭时程序不发出任何网络请求。
settings-section-ocr = OCR
settings-ocr-auto-copy = 识别后自动复制，不弹面板
settings-ocr-auto-copy-hint = 直达 OCR 热键和截图界面工具条上的 OCR：识别出的文字直接进剪贴板，用通知提示字数。工具条上的“翻译”仍打开结果面板。
settings-update-checking = 正在检查更新…
settings-update-up-to-date = 已是最新版本（{ $version }）
settings-update-available = 发现新版本 { $version }
settings-update-open-page = 打开发布页面
settings-update-failed = 检查更新失败：{ $error }
settings-version = 当前版本 { $version }
settings-section-saving = 自动存储
settings-save-dir = 存储目录
settings-save-dir-default = 默认目录（“图片”文件夹下的 BetterPrtSc）
settings-save-dir-custom = 自定义目录
settings-browse = 浏览…
settings-choose-folder = 选择自动存储目录
settings-filename-template = 文件命名
settings-filename-example = 示例：{ $name }（按固定的示例时间 2026-01-01 12:34:56 生成，不是当前时间）
settings-filename-help = 可用占位符：{ "{" }mode{ "}" }（捕获模式）、{ "{" }window_title{ "}" }（窗口标题），以及 { "{" }yyyy-MM-dd HH-mm-ss{ "}" } 这样的日期时间（yyyy、MM、dd、HH、mm、ss、fff 等）。
settings-output-format = 截图格式
settings-output-format-hint = PNG 和 WebP 无损；JPEG 体积小但有损、不支持透明；AVIF 体积最小，但编码较慢。录屏的格式在热键绑定里另行设置。
settings-section-recording = 录屏
settings-recording-hint = 这些是录屏的默认参数。每条录屏热键还可以在 config.toml 里单独覆盖（写法见配置文件开头的说明）。
settings-recording-frame-rate = 帧率
settings-recording-frame-rate-default = 按格式（MP4 30、GIF 15）
settings-recording-fps = { $fps } fps
settings-recording-show-cursor = 录制鼠标指针
settings-recording-mp4 = MP4
settings-recording-mp4-hint = GIF 不使用下面的编码、画质和声音设置。
settings-recording-codec = 视频编码
settings-recording-probing = 正在检测这台电脑是否支持…
settings-recording-codec-unsupported = 这台电脑不能用 { $codec } 编码，录制时会改用 H.264：{ $reason }
settings-recording-codec-unavailable = { $codec } 不可用：{ $reason }
settings-recording-codec-no-decoder = 这台电脑的系统播放器放不了 { $codec } 视频：需要在 Microsoft Store 安装对应的视频扩展（H.265 是“HEVC 视频扩展”，AV1 是“AV1 视频扩展”），或者换用其他播放器。
settings-recording-quality = 画质
settings-recording-quality-low = 低（文件小）
settings-recording-quality-standard = 标准
settings-recording-quality-high = 高（文件大）
settings-recording-quality-custom = 自定义码率
settings-recording-quality-hint = 档位按录制范围的大小和帧率算码率；H.265、AV1 同样画质用的码率更低。
settings-recording-system-audio = 录入系统声音
settings-recording-microphone = 录入麦克风
settings-recording-microphone-default = 系统默认麦克风
settings-recording-microphone-missing = 未连接的麦克风（{ $id }）
settings-recording-microphone-fallback = 选中的麦克风现在没有连接，录制时会改用系统默认麦克风。
settings-recording-microphones-failed = 无法列出麦克风：{ $error }
settings-recording-refresh = 刷新
settings-recording-audio-hint = 系统声音和麦克风混在同一条音轨里。录制时可以在控制条上临时开关它们，只影响这一段录屏。
settings-section-bindings = 热键绑定
settings-bindings-hint = 点击热键框后直接按下组合键即可录制；字母和数字键需要搭配 Ctrl、Alt 或 Win。
settings-binding-failed = 注册失败：{ $reason }
settings-binding-add = 添加热键
settings-binding-remove = 删除
settings-binding-hotkey = 热键
settings-binding-mode = 捕获模式
settings-binding-actions = 截后动作 / 功能
settings-binding-auto-save = 自动保存
settings-binding-needs-modifier = 字母和数字键需要搭配 Ctrl、Alt 或 Win
settings-config-broken = 配置文件有错误，无法在这里编辑
settings-config-broken-hint = 请用文本编辑器修正下面这个文件，然后点“重新读取”。
settings-config-not-utf8 = 配置文件不是 UTF-8 编码
settings-config-unreadable = 无法读取配置文件：{ $error }
settings-changed-elsewhere = 配置文件在打开设置后被其他程序修改过，为免覆盖那些修改，没有保存。可以“重新读取”后再改（会丢弃这里的修改）。
settings-fix-problems = 标红的修改不合法，没有保存；配置仍是上一次的有效值。
settings-reload = 重新读取
settings-open-config-folder = 打开配置文件夹
settings-saved = 已保存，立即生效。
settings-saved-not-running = 已保存，下次启动 BetterPrtSc 时生效。
settings-save-failed = 保存失败：{ $error }
settings-unsaved-title = 有未保存的修改
settings-unsaved-body = 有修改没能保存（不合法或写入失败），关闭后这些修改会丢失。
settings-discard-and-close = 仍然关闭
settings-cancel = 取消

## 选区遮罩
overlay-selected-hint = Enter 或双击选区：完成　Esc：取消
overlay-copy-color-hint = C：复制颜色
overlay-color-copied = 已复制颜色
overlay-color-copy-failed = 复制失败
overlay-record-hint = Enter 或双击选区：开始录屏　Esc：取消

## 录屏控制条
recording-toolbar-pause = 暂停
recording-toolbar-resume = 继续
recording-toolbar-stop = 停止
recording-toolbar-system-audio = 系统声音
recording-toolbar-microphone = 麦克风

## 标注

annotation-rectangle = 矩形
annotation-arrow = 箭头
annotation-mosaic = 马赛克
annotation-text = 文字
annotation-select = 选择
annotation-undo = 撤销
annotation-redo = 重做
annotation-cancel = 取消
annotation-save-as = 另存为
annotation-confirm = 确认
annotation-record = 录屏
annotation-long-screenshot = 长截屏
annotation-ocr = OCR
annotation-translate = 翻译
save-as-file-type = { $format } 图像
notification-save-as-failed-title = 另存为失败

## 结果面板（OCR）

result-panel-window-title = OCR 结果
result-panel-language = 识别语言
result-panel-recognizing = 正在识别…
result-panel-recognized = 识别完成
result-panel-no-text = 没有识别到文字
result-panel-failed = 识别失败：{ $error }
result-panel-no-language-pack-title = 系统没有可用的 OCR 语言包
result-panel-no-language-pack-body = BetterPrtSc 使用 Windows 自带的文字识别，需要系统装有对应语言的 OCR 组件。请打开系统设置的“时间和语言 > 语言和区域”，添加要识别的语言（例如“中文(简体，中国)”“English (United States)”），Windows 会一并安装它的 OCR 组件；已经添加的语言，可在它的“语言选项”里确认“光学字符识别”已安装。装好后点“重新识别”即可，无需重启。
result-panel-open-language-settings = 打开语言设置
result-panel-recognize-again = 重新识别
result-panel-copy = 复制
result-panel-copied = 已复制到剪贴板
result-panel-copy-failed = 复制失败：{ $error }
result-panel-auto-copy = OCR 识别后自动复制，不弹面板
result-panel-auto-copy-failed = 无法保存设置：{ $error }
result-panel-open-settings-failed = 无法打开系统设置：{ $error }
result-panel-close = 关闭
config-unknown-translation-provider = 未知的翻译服务“{ $provider }”：可用 openai（OpenAI 兼容接口）、deepl
config-unknown-target-language = 未知的翻译目标语言“{ $language }”：可用 auto（跟随界面语言）、zh-CN、en、ja、ko、fr、de、es、ru
translation-error-network = 无法连接翻译服务：{ $detail }
translation-error-rejected = 翻译服务拒绝了 API Key（HTTP { $status }），请检查 config.toml 里 [translation] 的 api_key
translation-error-http = 翻译服务返回了错误（HTTP { $status }）
translation-error-http-detail = 翻译服务返回了错误（HTTP { $status }）：{ $detail }
translation-error-reply = 无法理解翻译服务的回复
result-panel-translate = 翻译
result-panel-translate-unconfigured = 没有配置翻译服务：请在配置文件 config.toml 的 [translation] 里填写服务（openai 或 deepl）和你自己的 API Key
result-panel-translate-unconfigured-hint = 未配置翻译服务（见 config.toml 的 [translation]）
result-panel-translating = 正在翻译…
result-panel-translation = 译文
result-panel-translation-failed = 翻译失败：{ $error }
result-panel-copy-translation = 复制译文
## 长截屏

long-screenshot-start = 开始
long-screenshot-finish = 完成
long-screenshot-cancel = 取消
long-screenshot-size = 已拼接 { $width } × { $height }
long-screenshot-lost = 滚动太快，请往回滚一点
notification-long-screenshot-full-title = 长截屏已到达高度上限
notification-long-screenshot-full-body = 长截屏最高 { $limit } 像素，已自动停止拼接。

# bps 命令行
config-toml-error = 配置文件格式错误：{ $message }
cli-usage =
    用法：bps capture <捕获模式>

    把捕获命令转交给 BetterPrtSc 常驻实例执行，行为与按下对应热键一致：
    采用第一条该捕获模式的热键绑定的截后动作，没有这样的绑定时复制到剪贴板。
    常驻实例没在运行时会先启动它。
cli-usage-modes = 捕获模式：
cli-exit-codes = 退出码：
cli-exit-success = 成功
cli-exit-failed = 常驻实例执行命令失败
cli-exit-usage = 参数有误
cli-exit-unreachable = 无法连接或启动常驻实例
cli-exit-cancelled = 用户取消了这次捕获（例如在选区遮罩上按了 Esc）
cli-cancelled = 已取消
cli-failed = 执行失败：{ $message }
cli-unreachable = 无法把命令转交给常驻实例：{ $error }
cli-unknown-mode = 未知的捕获模式：{ $mode }
cli-missing-mode = 缺少捕获模式
cli-extra-argument = 多余的参数：{ $argument }
cli-missing-command = 缺少命令
cli-unknown-command = 未知的命令：{ $command }
