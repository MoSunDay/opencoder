Commit: (working-tree, pre-initial-commit)

# fix(session): steer/queue 提升消息标记为真实用户输入（非 synthetic）

## 背景
`record_compound`（steer/queue 提升路径）把所有文本消息标记为 `synthetic = true`，
而 idle Submit 路径标记为 `synthetic = false`。两条路径的文本处理完全一致（skill
解析、plan tag 注入），唯一区别就是这个标记。

这导致三个问题：
1. **resume 后已消费的 queue/steer 提交从 transcript 消失** — `replay.rs` 跳过
   synthetic 消息，消费 marker 是纯内存状态不持久化
2. **autopilot 目标提取失效** — `extract_goal` 过滤 `!m.synthetic`
3. **compaction turn 边界识别错误** — `turn_start_indices` 过滤 `!m.synthetic`

## 变更
### `record_compound` 文本路径改为非 synthetic
- **`crates/session/src/skill_resolve.rs:96`**：移除 `m.synthetic = true`（文本路径），
  与 idle Submit 路径一致。纯 skill trigger（`SKILL_TRIGGER`）仍保持 `synthetic = true`。
- **`crates/session/src/runner/mod.rs:230,239`**：注释从 "synthetic" 改为 "real user"。
- **`crates/tui/src/session_ui/replay.rs:26-30`**：更新注释说明 steer/queue 提升消息现在
  会在 resume 后渲染为 `user:` 块。

### 测试更新
- **`crates/session/src/skill_resolve.rs:196`**：断言从 `synthetic == true` 改为 `false`。
- **`crates/session/tests/plan_tag.rs`**：`synthetic_user_texts` → `promoted_user_texts`
  （按 `Role::User` 过滤 + skip kickoff）；新增 `kickoff_text` helper。
- **`crates/session/tests/steer_followup.rs:226,269`**：过滤从 `m.synthetic && ...` 改为
  `m.role == Role::User && ...`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 文本路径 non-synthetic | `record_compound_records_cleaned_text` | `skill_resolve.rs` |
| 纯 skill trigger 仍 synthetic | `record_compound_pure_skill_injects_trigger` | `skill_resolve.rs` |
| steer prompt tagged after first | `steer_prompt_tagged_after_first` | `plan_tag.rs` |
| queued prompt tagged after first | `queued_prompt_tagged_after_first` | `plan_tag.rs` |
| steer promoted into history | `steer_promotes_at_turn_boundary` | `steer_followup.rs` |
| multiple steers promoted | `multiple_steers_at_one_boundary_promoted_once` | `steer_followup.rs` |
| queue drains FIFO | `queue_drains_all_fifo_in_single_run_then_done` | `steer_followup.rs` |
| idle compound non-synthetic | `compound_command_from_idle_records_trailing_as_real_prompt` | `control_cmd.rs` |

- 全量回归：`cargo test --workspace` → 1830 passed / 0 failed / 1 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`skill_resolve.rs` 224 ≤ 800

## Impact Surface
- **resume 后可见性修复**：quit→resume 后，之前通过 queue/steer 提交的消息不再消失，
  会以 `user:` 块形式渲染在 transcript 中
- **autopilot goal 提取修复**：`extract_goal` 现在能正确找到第一个真实用户消息
- **compaction turn 边界修复**：steer/queue 提升现在被正确识别为新 turn 起点
- **live 行为不变**：提交时不回显（只进 pending panel），执行时先回显 marker 再调 LLM
- 不影响：handoff 消息（仍 synthetic）、compaction summary（仍 synthetic）、
  dangling tool error（仍 synthetic）、session title 提取（仍取首个非 synthetic）

## Related Docs
- [agents/session](../../agents/session/index.md)
- [既有相关 changelog](../2026-08-04/tui-steer-queue-echo-at-consume-not-admit.md)
- [既有相关 changelog](../2026-08-01/queued-combined-skill-display.md)
