---
status: accepted
---

# 截后动作改为“复制或等待确认”，自动保存独立成开关

此前（#5、#6、#12、#13 等）热键绑定的 `actions` 是一个可组合的截后动作列表（复制、自动存储、标注、OCR、翻译），按勾选顺序执行。试用反馈表明这个模型难以理解，且“标注”“自动存储”这些按钮混在标注工具条的结束方式里。现改为：

- `after` 二选一：`copy_to_clipboard`（复制后直接结束）或 `await_confirmation`（留在截图界面等用户确认；Enter / 确认复制，Esc / 取消不复制）。
- `auto_save` 是绑定上独立的布尔开关（默认 false），在最终交付时生效，取消不保存。
- `function = "record" | "long_screenshot" | "ocr"` 为直达功能，与 `after` 互斥。
- 等待确认的工具条可以转入录屏、长截屏、OCR、翻译和另存为。

旧的 `actions = [...]` 不做兼容：解析时给出明确的本地化错误通知，该绑定被忽略。

取代此前“截后动作是可组合列表”的约定；ADR 0006 的标注光栅化不受影响。

## Consequences

- `CaptureRequest` 由 `{ mode, actions }` 变为 `{ mode, kind, auto_save, .. }`，`kind` 为 `Screenshot(PostCaptureAction)` 或 `Function(DirectFunction)`。
- 工具条转入录屏、长截屏时沿用原请求，再按同一热键仍可停止录屏；窗口模式下录屏目标是该窗口。
- 结果面板新增 `--translate` 启动参数，识别完成后自动翻译。
