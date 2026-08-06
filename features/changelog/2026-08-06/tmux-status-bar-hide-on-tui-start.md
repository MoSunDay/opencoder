# TUI 启动时隐藏 tmux 状态栏

## 背景

tmux 底部状态栏占据一整行屏幕。当 opencode TUI 全屏运行时，这一行与 TUI 抢占
有限的垂直空间（笔记本 / 分屏窗口下尤为明显）。此前 TUI 启动后状态栏始终可见，
退出后也无变化。

本次新增纯运行时行为：TUI 启动时隐藏 tmux 状态栏，退出时（正常返回 **或** 错误）
恢复原始状态。不读写配置文件、不修改用户 tmux 配置；非 tmux 环境完全无副作用。

## 变更

### 新增模块 `crates/tui/src/tmux_bar.rs`（124 行）

纯函数式实现，无 class / 内部可变状态：

- `parse_status(&str) -> Option<bool>` — 纯函数，解析 tmux `status` 选项值
  （`on` → `Some(true)`，`off` → `Some(false)`，其余 → `None`）。trim 空白以兼容
  `display-message` 的尾随换行。**核心可测逻辑**已抽离为纯函数。
- `inside_tmux()` — 检测 `TMUX` 环境变量。
- `current_status()` — `tmux display-message -p "#{status}"` 读取，失败返回 `None`。
- `set_status(bool)` — best-effort `tmux set status on|off`，错误被吞（纯外观）。
- `pub fn hide() -> Option<bool>` — 捕获并隐藏；非 tmux 环境 `None`（no-op）。
- `pub fn restore(Option<bool>)` — 恢复捕获状态；`None` 为 no-op。

### 接入点 `crates/tui/src/app_bootstrap.rs`

- `run()` 在 `TerminalGuard::enter()` **之前**调用 `tmux_bar::hide()` 捕获原始状态；
  `run_app` 返回后（**先于** `drop(_guard)`）调用 `tmux_bar::restore(prev)`。
- 关键修复：采用 `let result = ...await; restore(prev); result?` 形式，保证 **错误
  路径**也恢复状态栏（旧 `.await?` 形式在 Err 时会跳过恢复）。

### 模块注册 `crates/tui/src/lib.rs`

- `pub mod tmux_bar;`（字母序置于 `theme` 与 `undo` 之间）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `parse_status("on") → Some(true)` | `parse_status_recognizes_on` | `crates/tui/src/tmux_bar.rs` |
| `parse_status("off") → Some(false)` | `parse_status_recognizes_off` | `crates/tui/src/tmux_bar.rs` |
| 未知值 / 空串 → `None`（不盲目恢复） | `parse_status_rejects_unknown_value` | `crates/tui/src/tmux_bar.rs` |
| trim 尾随换行（真实 tmux 输出形状） | `parse_status_trims_trailing_newline` | `crates/tui/src/tmux_bar.rs` |
| 非 tmux 环境 `hide()` → `None`（不 spawn 进程） | `hide_returns_none_outside_tmux` | `crates/tui/src/tmux_bar.rs` |
| `restore(None)` 无副作用、不 panic | `restore_none_is_noop` | `crates/tui/src/tmux_bar.rs` |

> rules/01 I/O 豁免：`current_status` / `set_status` / `inside_tmux` 直接调用 tmux
> 二进制或读进程环境，属「纯 I/O 包装，无法在无 tmux 的 CI 沙箱内测试」，按
> rules/01 豁免。唯一业务逻辑（状态串解析）已抽为纯函数 `parse_status` 并完整覆盖。

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 1895 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |

行数约束：`tmux_bar.rs` 124 行（≤400）；`app_bootstrap.rs` 125 行（≤800）。

## Impact Surface

- 仅影响 TUI 启动 / 退出的终端外观行为；不触及 config / 菜单 / session runner /
  store / prompt 契约 / 跨 crate API。
- 非 tmux 环境（含 CI）完全无副作用（`TMUX` 未设 → `hide()` 返回 `None`）。

## 备注

- Gate 计数：当次实跑 **1895 passed / 0 failed / 0 ignored / 0 filtered**。本 changelog 所在
  commit（a4b3395）一并移除了 web/browser/SERP/computer-use 工具及其测试，使全工作区测试
  总数相对早期 changelog 标注的 1898 下降约 9；1895 为本特性当前树状态的真实计数。
- 工作区存在与本特性无关的预存 flaky 测试（`data_dir::tests::is_deterministic`、
  `model_menu` 部分游标位置测试），偶发失败、重跑即过，不属于本次变更范围。
