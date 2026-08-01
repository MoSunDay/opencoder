Commit: (working-tree, pre-initial-commit)

# fix(tui): TUI 模式切换（SwitchAgent）落库，quit→resume 后保持切换后的 agent

## 背景

TUI 内切换 agent 模式（`/act`、`/plan` 等，经 `UiCmd::SwitchAgent` /
`SwitchAgentNoClear` / `SwitchAndStart`）此前只在内存 SessionState 生效：
worker 仅广播 `SessionEvent::AgentSwitch`，从不写 store。quit→resume 或 `/task`
picker 重载时 `sessions.agent` 仍是旧值，模式切换"丢"在重启边界（plan→act
移交场景尤其明显）。

## 变更

### 模式切换持久化（`crates/session/src/control_cmd.rs`、`crates/tui/src/worker.rs`）

- **`control_cmd.rs`**：`persist_agent` 由私有改为 `pub`——best-effort 写
  `SessionPatch { agent, updated_at }` 到 `sessions.agent`（沿用 `persist_clear`
  既有模式）。
- **`worker.rs`**：`SwitchAgent` 与 `SwitchAgentNoClear` 两条处理分支在广播
  `SessionEvent::AgentSwitch` 后调用 `persist_session_agent(sess, &name)`（事件
  文本 clone 避免借用冲突）；`SwitchAndStart` 已由既有 handoff 持久化覆盖。
- resume 侧零改动：`resume()` 本就按 `sessions.agent` 重建 SessionState，落库后
  `/task` picker 与重启恢复读取到切换后的模式。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 切换模式并 resume 后保持 | `switch_agent_persists_mode_and_survives_resume` | crates/tui/tests/agent_switch_persist.rs |
| SwitchAndStart（plan→act）落库 act | `switch_and_start_handoff_persists_act_mode` | crates/tui/tests/agent_switch_persist.rs |

- 全量回归：`cargo test --workspace` → 102 binaries，**1587 passed / 0 failed / 1 ignored**（当次实跑；同 commit 的 queue 面板滚动 12 个测试一并计入，见 [tui-queue-panel-scroll](./tui-queue-panel-scroll.md)）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`worker.rs` 792 ≤ 800（迭代）；`control_cmd.rs` 301 ≤ 800；`agent_switch_persist.rs` 179 ≤ 400（新文件）

## Impact Surface

- **可感知影响**：TUI 内切换 agent 模式后，quit→resume 与 `/task` picker 读取
  切换后的模式；plan→act 移交跨重启保持。
- **不影响**：web / CLI headless（本就走 admit 侧持久化）、skill 机制、消息/事件
  存储形状。

## Related Docs

- [agents/session](../../../agents/session/index.md)
- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](../2026-07-31/fix-tui-task-switch-cancel-before-quit.md)
