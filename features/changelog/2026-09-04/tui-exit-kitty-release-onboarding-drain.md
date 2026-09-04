Commit: 53519a1

# TUI 退出残留 `0;5:3u` 乱码：onboarding 退出路径补 input drain

## Context

不开 tmux 时 TUI 退出后 shell 残留 `0;5:3u`（kitty keyboard protocol 的 release report）。`0;5:3u` 是 Ctrl+D 按键释放时终端发来的 CSI u 序列：TUI 退出（禁用 kitty 协议、恢复 cooked mode）后这条 report 才到达 shell，被 shell 当作输入回显。

`run_app`（正常路径）退出前调用 `input::drain_shutdown`（DRAIN_QUIET=80ms 静默窗吸收延迟 report）。排查发现 onboarding 路径漏掉：全新 HOME 首次启动走 `onboarding::run`，用户 Ctrl+D 退出时 `app_bootstrap.rs` 直接 `return`，不经过 drain → report 泄漏到 shell。HEAD 的 pty+pyte 探针实证：主路径干净、onboarding 路径必现。

## 变更

- **tui**（`onboarding.rs`）：`Exit` 分支在 `drop(input_rx)` 前补 `input::drain_shutdown(&mut input_rx,Instant::now())`，与 `run_app` 退出序列对齐。

## 测试

| 场景 | 用例 | 位置 |
|---|---|---|
| onboarding 退出吸收延迟的 kitty release report（5/15/30/60ms 全档） | `onboarding_quit_absorbs_delayed_kitty_release_reports` | tests/tui_exit_restore_e2e.rs（pty+pyte e2e） |

回归：worktree 下 `cargo test -p opencoder --test tui_exit_restore_e2e` 4/4、`cargo test -p opencoder-tui --lib` 全量绿。150ms 级延迟超 DRAIN_QUIET 上界仍会漏（既有取舍，非本次范围）。
