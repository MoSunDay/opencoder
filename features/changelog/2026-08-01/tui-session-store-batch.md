Commit: (working-tree, pre-initial-commit)

# feat(tui/session/store): agent 模式持久化 + 排队 skill 展示 + /task skill 标签（本批汇总）

## 背景
本提交为一批已完成开发、经 review 的变更汇总提交，涵盖 TUI 交互三块功能及其
在 session/store/web 的支撑改动。测试按人工授权免测执行（未跑 cargo test/clippy）。

## 变更
### fix(tui): agent 模式切换落库
- **`crates/session/src/control_cmd.rs`**：`persist_agent` 改为 `pub`，TUI worker
  在 `UiCmd::SwitchAgent` 路径调用，quit→resume / `/task` 重载读到的 agent 为切换后值。
- 细节见 `tui-agent-mode-persist.md`。

### fix(tui): 排队提交 skill 展示闭环
- **`crates/store/src/libsql_store/schema.rs` / `inputs.rs` / `types.rs`**：`session_inputs`
  新增 `display_text` 列（schema v6 迁移），排队/插队项保留含 `{$skill}` token 的原文，
  drain 仍只消费 clean `prompt`。
- 细节见 `queued-combined-skill-display.md`。

### feat(tui): /task 选择器 skill 标签
- **`crates/store/src/libsql_store/sessions.rs`**：`list_sessions` 返回 `sessions.skill` 列，
  TUI `/task` 选择器按行渲染激活 skill。
- 细节见 `tui-task-picker-skill-tag.md`。

### test(store): 测试拆分与迁移覆盖
- **`crates/store/tests/store_migrations.rs` / `store_concurrency.rs` / `display_text.rs` /
  `subagent_status_counts.rs`**（新增）：从 `store_integration.rs`（813 行 → 缩减）拆出
  迁移、并发、display_text、subagent 计数四组独立测试。

### web / memory
- **`crates/web/src/api.rs` / `handle.rs`**：与上述功能对应的最小接线。
- **`agents/store/index.md`、`agents/tui/index.md`**：memory repair-on-touch。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| display_text 迁移/展示 | display_text 组 | `crates/store/tests/display_text.rs` |
| store 并发 | store_concurrency 组 | `crates/store/tests/store_concurrency.rs` |
| store 迁移矩阵 | store_migrations 组 | `crates/store/tests/store_migrations.rs` |
| subagent 状态计数 | subagent_status_counts 组 | `crates/store/tests/subagent_status_counts.rs` |
| 排队 skill drain | queued_skill_drain | `crates/tui/tests/queued_skill_drain.rs` |
| resume 队列展示 | resume_queue_display | `crates/tui/tests/resume_queue_display.rs` |
| agent 切换持久化 | agent_switch_persist | `crates/tui/tests/agent_switch_persist.rs` |

- 全量回归：`cargo test --workspace` → **未执行（人工授权免测）**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **未执行（人工授权免测）**
- 行数：`store_integration.rs` 缩减至 <800；新增测试文件均 ≤400

## Impact Surface
- TUI：quit→resume 后 agent 模式保持；排队 skill 展示不再消失；/task 显示 skill 标签。
- store：`session_inputs` v6 迁移（新增 `display_text` 列，旧库自动迁移）。
- 不影响：CLI/Web drain 行为（仍消费 clean prompt）。

## Related Docs
- [agents/tui](../../../agents/tui/index.md)
- [agents/store](../../../agents/store/index.md)
- [既有相关 changelog](../2026-08-01/queued-combined-skill-display.md)
