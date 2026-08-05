Commit: (working-tree, pre-initial-commit)

# fix(tui): reconcile status chip when AgentSwitch event is dropped

## 背景
TUI 中 `[plan]`/`[act]` 状态标签在 plan→act 切换后可能卡在 `[plan]`。根因：`worker.rs::forward_event` 使用 `try_send` 向 512 容量的 UI channel 转发 `AgentSwitch` 事件，channel 饱和时 `Err(Full)` 被 `let _ =` 静默丢弃。`chat.agent` 字段仅由 `AgentSwitch` 事件写入，一旦丢失便永久过期。

## 变更
### 双层修复
- **`crates/tui/src/app_loop.rs`**：`handle_switch_agent` 在派发 `UiCmd` 前乐观写入 `chat.agent = name.clone()`，覆盖不产生 TurnDone 的非流转切换（如 Alt+Tab）。
- **`crates/tui/src/worker.rs`**：`UiEvent::TurnDone` 从单元变体改为 `TurnDone(String)`，携带 `sess.agent.name.clone()`。`fold_ui_events` 在收到 `TurnDone(agent)` 时执行 `chat.agent = agent` 对账。TurnDone 走 `send().await`（必定送达），AgentSwitch 走 `try_send`（可能丢），两者互为补充。
- **`crates/tui/src/worker.rs`**：修正 `forward_event` 注释，删除"always get through"误导性描述。

### 测试
- **`crates/tui/src/app_loop_bugfix_tests.rs`**：新增 `turn_done_reconciles_agent_when_agent_switch_dropped` 和 `handle_switch_agent_sets_agent_optimistically` 两条回归测试。
- **`crates/tui/src/worker/tests.rs`** / **`tests_reload.rs`**：适配 `TurnDone(String)` 新签名。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| TurnDone 对账 | turn_done_reconciles_agent_when_agent_switch_dropped | app_loop_bugfix_tests.rs |
| 乐观更新 | handle_switch_agent_sets_agent_optimistically | app_loop_bugfix_tests.rs |
| TurnDone 签名适配 | (既有 worker tests) | worker/tests.rs, worker/tests_reload.rs |

- 全量回归：`cargo test --workspace` → 890 passed; 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：worker.rs < 800, app_loop.rs < 800, app_loop_bugfix_tests.rs < 800

## Impact Surface
- 对用户：修复状态标签卡死问题，plan/act 切换后 UI 立即正确反映当前 agent。
- 不影响：CLI、Web、session 运行时、store 层、LLM 调用。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
