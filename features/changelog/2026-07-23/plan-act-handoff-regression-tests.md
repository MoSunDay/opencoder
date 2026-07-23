Commit: (working-tree, pre-initial-commit)

# test(session,tui): plan→act handoff regression test coverage

## 背景
commit `83fc3e6`（`plan_submitted` guard）修复了 plan→act 切换后消息重复 /
steer 消费不幂等的问题。当时未将验证这些路径的回归测试纳入提交。本次补齐。

## 变更
### session crate 集成测试
- **`crates/session/tests/plan_act_dup_check.rs`**（3 tests）：
  - `handoff_run_no_duplicate`：handoff → run("") 后消息数 = 3 且无重复 id
  - `handoff_resume_run_no_duplicate`：handoff → run → resume（不新增消息）→ run
  - `handoff_steer_consumed_once`：steer 在 handoff 跨界时仅消费一次

### tui crate 集成测试
- **`crates/tui/tests/plan_card_full_flow.rs`**（2 tests）：
  - AgentSwitch 单独不生成 Plan block；TranscriptReset replay 后恰好一个 Plan block；
    PlanHandoff 在已 replay 的 ChatView 上正确触发
  - 含 clippy 修复 L138：`std::slice::from_ref(&handoff_msg)` 替代 `&[handoff_msg.clone()]`

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| handoff 后无消息重复 | `handoff_run_no_duplicate` | plan_act_dup_check.rs |
| resume 不新增消息 | `handoff_resume_run_no_duplicate` | plan_act_dup_check.rs |
| steer 单次消费 | `handoff_steer_consumed_once` | plan_act_dup_check.rs |
| AgentSwitch 不建 Plan block | `forward_order_...` / `reverse_order_...` | plan_card_full_flow.rs |

- 行数：plan_act_dup_check.rs 122（< 400）、plan_card_full_flow.rs 187（< 400）

## Impact Surface
- 仅新增测试文件，不改生产代码。无行为变化。

## Related Docs
- [commit 83fc3e6](../../) plan→act handoff `plan_submitted` guard
