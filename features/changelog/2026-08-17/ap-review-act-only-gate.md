Commit: (working-tree, post-5ac8cc9)

# autopilot review pass 的 act-only 门控

## Context

`autopilot.mode=review` 此前无 primary agent 门槛：初始任务完成后无论当前 primary session 跑的是哪个 agent，runner 都会发起一次性 review pass（切 plan agent + 激活 review skill + 合成 review prompt）。当用户以 plan 模式（或任何非 act primary）工作时，这会在**未执行任何实现**的会话上误触发 review——评的是一份没被执行过的 plan，产出无意义且打断了 plan 模式的纯咨询语义。

## Change Summary

- `crates/session/src/runner/mod.rs`（run 尾部 dispatch，净 +4 行）：`ApMode::Review` arm 追加 match guard `if session.agent.kind == AgentKind::Act`——review 评的是**已执行的工作**，只有 act primary 才分发；plan 模式等非 act primary 落入与 `off` 相同的空 arm（`Off | Review => {}`），不发起、不切 agent、不激活 skill、不注入合成 review prompt。`Ap` 路径与 todos workflow 强制 off（parent.rs / execution.rs）均不受影响。
- `crates/session/src/autopilot/review_pass.rs`：模块头注释补 act-only gate 段（该文件其余改动属并发 queue-drain 会话，非本轮产物）。
- `crates/session/tests/autopilot_review.rs`（+175/-11，删除行均为排版 reflow，断言无弱化）：新增负路径测试 `review_mode_skips_pass_in_plan_mode`。
- 记忆文档 repair-on-touch：`features/index.md` 与 `agents/session/index.md` 各补 1 处 act-only 门控语义。

## Validation（当次实跑）

- `cargo test -p opencoder-session --test autopilot_review --test autopilot --test control_cmd`：autopilot_review **5 passed / 0 failed**、autopilot **23 passed / 0 failed**、control_cmd **15 passed / 0 failed**。
- `cargo build --workspace`：Finished dev profile。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（Finished dev profile）。并发会话在途文件遗留的两项诊断（tui `menu.rs` bool_comparison、tests/common unused import）已按 clippy 自动建议做最小修复（各 1 行，属并发 diff 的 lint 合规，非语义改动）。
- `cargo test --workspace`：**2855 passed / 0 failed**（exit 0；回归基线 2803 + 本轮新增 1 + 并发会话增量）。首轮曾现 session lib 1 失败，复跑全绿——系并发在途编辑的瞬态，非本轮 diff。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `autopilot_review.rs::review_mode_skips_pass_in_plan_mode` | integration | **负路径（本轮新增）**：plan 模式下不发起 pass——四面断言：mock 单脚本耗尽即炸的防误触发设计（`call_count==1` 仅初始 turn）、零 `AutoPilot` 事件、无 `AgentSwitch`、无 skill 激活/残留（skill_prompt 与 active_skill_names 均空）、无 synthetic 消息落入 transcript |
| `autopilot_review.rs::review_mode_runs_exactly_one_review_pass` | integration | 正路径：act primary 下恰好一次 review turn，事件 `AutoPilot{phase:Review}`，不进 ACT/VERIFY |
| `autopilot_review.rs::review_mode_activates_then_clears_review_skill` | integration | 正路径：review skill 激活 → pass 结束清空，无残留 |
| `autopilot_review.rs::ap_mode_with_max_iterations_one_still_cycles_phases` | integration | 回归：门控改动不影响 `Ap` 模式单迭代 PLAN→ACT→VERIFY 循环 |

结构性变更覆盖：门控两分支（Act 放行 / 非 Act 落空）各有测试锁定；无删测试 / 无 `#[ignore]` / 无弱断言。
