Commit: (working-tree, post-3320cbb)

# 修复 plan→act 交接双回归：ts-origin 重武装失效 + 阶段重置后 /act_clear_context 清空全部

## Context

两个失效同源：ecce7b0 引入的「消费时点重武装 + 阶段有界快照」有两个门控缺陷，在
ts-origin 会话（session 行 `agent` 列恒 NULL，见 `lib.rs::ts_origin`）与手工切回
plan（`reset_plan_phase` 清空快照+计数）两条路径上把真实计划交接降级：

1. **Bug 1**：`app_loop.rs` TurnDone(plan) 重武装条件含 `meta.agent == Some("plan")`
   合取。ts-origin 行 agent 为 NULL → Enter 提交时的乐观武装被覆盖成 false →
   `chat.plan_submitted` 为假 → **Shift+Tab 只切模式、不交接计划**。
2. **Bug 2**：手工切回 plan 时 `control_cmd.rs` 调 `reset_plan_phase()` 清掉
   `plan_input_count` + `plan_snapshot` 并落库；`plan_handoff::handoff()` 只读快照，
   快照没了 → 返回 None → `/act_clear_context` 走 blank sentinel **清空全部上下文**。
   另有 legacy 会话（plan-phase 列加列前创建，counter=0 / snapshot=NULL）resume 后
   同样未武装，且回填门带 `agent == Some("plan")` 合取把 agent=NULL 的 ts 行排除在外。

## Change Summary

- **`crates/tui/src/app_loop.rs`**：TurnDone(plan) 重武装去掉 `meta.agent` 合取，改为
  `plan_input_count > 0 || plan_snapshot.is_some()`。TurnDone("plan") 事件本身即证明
  plan agent 刚跑完一个 turn，agent 列是否 NULL 无关；与 `app_helpers.rs`、
  `app_task.rs` 既有口径一致。
- **`crates/session/src/plan_handoff.rs`**：新增 `newest_plan_agent_text(&[Message])`：
  最新 assistant、`agent == Some("plan")`、非 synthetic、非空（消息级阶段边界——
  runner 为每条 assistant 消息写 agent 标签，act 答案 tag 为 "act" 永不误取）。
  `handoff()` 改为 `plan_snapshot.clone().or_else(|| newest_plan_agent_text(...))`：
  快照缺失时回退到 transcript 里的 plan 标记文本，防伪造保证不破。
- **`crates/session/src/resume.rs`**：legacy 回填门改为
  `meta.agent.as_deref().is_none_or(|a| a == "plan")`（NULL 放行、显式 act 拒绝），
  回填实现复用 `newest_plan_agent_text`。
- **`crates/session/src/control_cmd.rs` / `crates/tui/src/worker.rs` /
  `crates/tui/src/app_helpers.rs` / `crates/tui/src/app_task.rs`**：各 plan 武装门从
  `plan_input_count > 0` 扩为 `|| plan_snapshot.is_some()`，快照回填会话同样武装。

## 反伪造保证（不回归 ecce7b0）

消息级回退只认 `agent == "plan"` 标签：act 阶段回答 tag 为 "act" 绝不可能被包装成
计划；synthetic（compaction 摘要 / handoff 指令）与空文本被跳过。残余风险（同阶段
plan turn 零输出失败时回退取到更早 phase 的 plan 标记答案）与 snapshot 语义等价且
需极端流程。

## 测试清单

| 测试 | 层 | 断言 |
|---|---|---|
| `tui app_loop_tests/mod.rs::fold_turn_done_plan_rearms_from_persisted_counter` | unit | 新增 `agent: None + count: 2`（ts-origin）与 `snapshot-only` 两用例：NULL agent 也武装、快照单独武装；`agent: Some("plan") + count: 0` 仍解除武装 |
| `tui app_loop_act_clear_ts_origin_tests.rs::shift_tab_ts_origin_session_hands_plan_forward` | integration | ts-origin 行（agent=None）+ `ts_origin()` 会话：真实 plan turn 后 Shift+Tab 必须发 `SwitchAndStart`（用户原场景复现） |
| `tui app_loop_act_clear_repro_tests.rs::shift_tab_after_real_plan_turn_hands_plan_forward` | integration | 常规 plan turn 后 Shift+Tab 交接、transcript 折叠为单条计划消息 |
| `tui app_loop_act_clear_repro_tests.rs::act_clear_context_after_real_plan_turn_preserves_plan` | integration | plan 模式 `/act_clear_context` 走 handoff（SwitchAndStart） |
| `session tests/control_cmd.rs::clear_context_plan_mode_keeps_plan_after_phase_reset` | integration | 快照被 reset（counter=0/snapshot=None）但 transcript 含 plan 标记消息 → `/act_clear_context` 保留计划交接（非 sentinel） |
| `session tests/control_cmd.rs::clear_context_act_mode_no_plan_still_blank_fresh_start` | integration | act 无计划 → 仍走全清空 sentinel（防伪造回归） |
| `session plan_handoff.rs::newest_plan_agent_text_*` | unit | 新 helper 取最新 plan 标记 assistant；跳过 act / synthetic / 空 / 无 assistant |
| `session tests/resume_legacy_plan_backfill.rs::legacy_ts_origin_null_agent_session_backfills` | integration | agent=NULL 行回填 counter=1 + 快照；既有 act 会话/失败 phase/持久态不覆盖用例全绿 |
| 回归：`cargo test --workspace`（web `client_echo_matches_server_persisted_events` 为基线既有失败，clean HEAD 复现）+ `cargo clippy --workspace --all-targets` | regression | 除基线既有失败外全绿 |

## Validation（当次实跑）

- `cargo fmt --all` 无改动残留；`cargo clippy -p opencoder-session` 0 warning（`map_or` →
  `is_none_or` 采纳 clippy 建议）。
- `cargo test -p opencoder-session`：lib 379 + 全部集成套件全绿（含新增 5 例）。
- `cargo test -p opencoder-tui`：lib 1445 全绿（含 act_clear_repro 3 例、rearm 新用例）。
- `cargo test -p opencoder-client -p opencoder-cli -p opencoder-todos` 全绿；
  `cargo test -p opencoder-web` 除基线既有失败外全绿。
