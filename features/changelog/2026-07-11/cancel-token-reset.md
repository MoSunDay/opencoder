Commit: (working-tree, pre-initial-commit)

# 双击 Esc 中止后再提交无响应 — cancel token 不刷新，run_loop 顶部 is_cancelled() 永真

## 背景
双击 Esc 中止一个运行中的任务后，再次输入并回车提交，会话毫无反应——既不报错也不执行，看起来提交被「吞掉」。

## 根因
`app.rs` 的双击 Esc 走 `KeyAction::Cancel` → `cancel.cancel()`。这个 `CancellationToken` 一旦取消便永久取消，且在整个 `run_app` 生命周期里只创建一次（`run_app` 开头 `CancellationToken::new()`），从未刷新。

提交新任务时 loop 发 `UiCmd::Prompt` → worker 调 `run` → `run_loop` 开头的第一条指令就是：
```rust
if let Some(c) = &session.cancel {
    if c.is_cancelled() {
        on_event(SessionEvent::Status("interrupted".into()));
        break;   // ← 永远在这里 break
    }
}
```
因为 `sess.cancel` 还是那个已取消的旧 token，`is_cancelled()` 永真，turn 在产出任何模型内容前就被打断——只发一条 `Status("interrupted")`，UI 上表现为「没反应」。

## 变更
- **`crates/tui/src/worker.rs`**：新增 `UiCmd::ResetCancel(CancellationToken)`；`process_cmd` 加分支 `sess.cancel = Some(c)`。
- **`crates/tui/src/app.rs`**：新增 `start_turn(cmd_tx, &mut cancel, cmd)` 辅助——建全新 token → 重赋 loop 的 `cancel` 句柄 → 按 mpsc FIFO 先发 `ResetCancel` 再发工作命令。在 4 个「发起 turn」的派发点替换直连 `cmd_tx.send(...)`：①`Submit` idle 分支 `Prompt` ②`SwitchAgent` plan→act 的 `SwitchAndStart` ③`/compact` 的 `Compact` ④`TurnDone` 自动续跑 `Prompt`。
- **不变项**：`Submit` running 分支只入队（store），不发工作命令，不刷新；`/task` 切换自带全新 token（`rebind_session`）；`rebind_session` 行为不变。loop 的 `cancel` 与 worker 的 `sess.cancel` 始终指向同一 token，双击 Esc 仍能命中当前 turn。
- 顺带修复工作区里**预先存在**、阻塞全量编译的 `compaction.rs:69` 借用错误（`m.text().as_str()` 引用了临时值）：把 `m.text()` 绑定到局部变量再 `strip_prefix/unwrap_or`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `ResetCancel` 把 `sess.cancel` 换成未取消的新 token | `reset_cancel_replaces_with_fresh_uncancelled_token` | `crates/tui/src/worker.rs` |
| 取消的 token 跳过 turn；重置后同一 session 正常跑出回复 | `cancelled_token_skips_turn_then_reset_lets_it_run` | `crates/session/tests/cancel_reset.rs` |
| 双击 Esc 硬中止仍即时（既有回归） | `cancel_hard_aborts_a_running_tool` | `crates/session/tests/hard_abort.rs` |
| `/task` 切换后双击 Esc 仍命中新会话 token（既有回归） | `rebind_session_swaps_the_active_cancel_token` | `crates/tui/src/worker.rs` |

- 全量回归：`cargo test --workspace` → 271 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 262 passed | 271 passed |
