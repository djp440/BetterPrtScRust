# BetterPrtScRust

## 代码结构

Cargo workspace（ADR 0002），所有触发方式都先转换成 `bps_core::Command` 再交给 `core` 执行：

- `crates/core`（`bps-core`）：平台无关逻辑，入口 `ResidentInstance`；端到端测试在 `crates/core/tests/`，用假平台从命令入口驱动
- `crates/platform`（`bps-platform`）：只定义能力 trait 和共用数据类型，`Platform` 是能力集合
- `crates/platform-fake`（`bps-platform-fake`）：测试用整套假平台，`bps-core` 的 dev-dependency
- `crates/platform-windows`（`bps-platform-windows`）：trait 的 Windows 实现，尽量薄，只有 `#[ignore]` 的真机冒烟测试
- `crates/app`（`bps-app`）：产出常驻实例 `BetterPrtSc.exe`，只做组装

## 常用命令

- 构建与测试：`cargo build --workspace`、`cargo test --workspace`（不碰真实桌面）
- 真机冒烟：`cargo test -p bps-platform-windows -- --ignored`（真实截屏、注册热键 Ctrl+F24 并模拟按下）
- 常驻实例端到端冒烟：`powershell -NoProfile -STA -File scripts\smoke-resident.ps1 [-Exe <路径>] [-ExitVia Tray]`
- 运行常驻实例：`cargo run -p bps-app`（debug 版带控制台输出；托盘菜单“退出”结束）
- 发布：推送 `v<版本>` tag 触发 `.github/workflows/release.yml`（tag 须等于根 `Cargo.toml` 的 `[workspace.package] version`）。本地按同样顺序运行 `scripts/release/` 下的 `package.ps1`（没装 Inno Setup 加 `-SkipInstaller`）→ `check-size.ps1 -Path dist\*-setup.exe, dist\*-portable.zip` → `measure-memory.ps1`；脚本自测 `test-release.ps1`

## Agent skills

### Issue tracker

Issue 和 spec 记录在本仓库的 GitHub Issues 中，使用 `gh` CLI 操作。见 `docs/agents/issue-tracker.md`。

### Triage labels

使用五个默认 triage 标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`。见 `docs/agents/triage-labels.md`。

### Domain docs

single-context 布局：根目录 `GLOSSARY.md` + `docs/adr/`。见 `docs/agents/domain.md`。
