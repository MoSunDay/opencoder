# fix(autopilot): VERIFY 判定方向修正 + Cancelled 独立 + 配置 clamp + 快照截断

## Summary

autopilot 全面审查后的四项修正：

1. **VERIFY 成功判定方向反转**：问题从「is MORE work needed?」（`no`=完成）改为
   正向设问「is the goal fully achieved?」（`yes`=完成）。原设问使 judge 顺着直觉答
   `yes, achieved` 时被解析为 MoreWork 继续空转，天然偏向永不完成。
2. **独立 `ApOutcome::Cancelled`**：会话 cancel 不再伪装成 `MaxIterations`；所有非
   Complete 终止路径（Cancelled / MaxIterations / Aborted）统一清 skill + emit `Done`。
3. **文档对齐**：`Aborted` 注释去掉错误的「or a phase run errored」（phase 错误经
   `?` 上抛，不折叠进 Aborted）；07-29 changelog 循环图修正为 PLAN 先行（原图先
   VERIFY，与实际实现不符）。
4. **健壮性**：`max_iterations=0` / `verify_retries=0` 在 `drive` 入口 clamp 到 1；
   VERIFY 快照按 `context_limit - 2_000` 预算截断到最近消息（防止多轮迭代后超出
   small model 窗口）。
- **测试健壮性**：`drive_returns_cancelled_when_cancelled_during_act` 的取消等待由固定 200ms sleep 改为事件驱动（轮询 mock call_count==2，即 ACT 的 bash 工具已执行后再 cancel），消除慢 CI 上 cancel 早于 ACT 导致 call_count==1 的抖动。

## Changes

### `crates/session/src/autopilot/prompts.rs`
- `verify_system_prompt` / `verify_user_prompt`：改为「is the goal fully achieved?」，
  `yes`=achieved/complete，`no`=more work needed。

### `crates/session/src/autopilot/decision.rs`
- `parse_verdict` 布尔语义文档更新：`true` = 对正向设问的肯定回答（= 目标已达成），
  token 解析映射不变（yes/y/true/1/是 → true）。

### `crates/session/src/autopilot/verify.rs`
- `Some(true)` → `VerifyVerdict::Complete`，`Some(false)` → `MoreWork`（原相反）。
- 新增 `build_snapshot`：快照预算 = `context_limit - 2_000`（`VERIFY_RESERVED_TOKENS`），
  超出时按 token 估算从最新消息往回贪心保留，目标经 question 重新陈述不丢失。

### `crates/session/src/autopilot/state.rs`
- 新增 `ApOutcome::Cancelled`。
- `Aborted` 注释修正（不含 phase-run error）；`VerifyVerdict` 注释随方向翻转。

### `crates/session/src/autopilot/mod.rs`
- `drive`：clamp `max_iterations`/`verify_retries` ≥ 1；`loop` 结构下取消检查返回
  `Cancelled`（loop 顶 / PLAN 后 / ACT 后三处）；所有终止路径走 `finish`（清 skill +
  emit `Done`）。

### 文档
- `features/changelog/2026-07-29/autopilot-review-skill-handoff.md`：循环图改为
  PLAN 先行。
- `features/changelog/2026-07-28/autopilot-loop.md`：终止条件与集成测试表随方向翻转。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| VERIFY=yes → Complete 且不污染 transcript | `verify_yes_means_complete_and_does_not_pollute_transcript` | `session/tests/autopilot.rs` |
| VERIFY=no → MoreWork | `verify_no_means_more_work` | 同上 |
| 垃圾裁决重试后 malformed | `verify_garbage_retries_then_malformed` | 同上 |
| 重试直到可解析 | `verify_retries_until_a_parseable_answer` | 同上 |
| drive 在 VERIFY=yes 完成循环 | `drive_completes_when_verify_says_yes` | 同上 |
| 阶段进度事件 Plan→Act→Verify | `drive_emits_autopilot_phase_events` | 同上 |
| malformed → Aborted | `drive_aborts_when_verify_keeps_malformed` | 同上 |
| max=1 + MoreWork → MaxIterations | `drive_max_iterations_one_yields_max_iterations` | 同上 |
| 预取消 → Cancelled（0 次 LLM 调用 + Done） | `drive_returns_cancelled_when_session_cancelled_before_loop` | 同上 |
| ACT 中取消 → Cancelled（不发 VERIFY） | `drive_returns_cancelled_when_cancelled_during_act` | 同上 |
| max_iterations=0 clamp 到 1 | `drive_clamps_zero_max_iterations_to_one` | 同上 |
| verify_retries=0 clamp 到 1 | `verify_retries_zero_is_clamped_to_one` | 同上 |
| 快照超窗截断（保留最新 + goal 问题） | `verify_snapshot_truncates_transcript_to_window` | 同上 |
| 关闭时不触发 drive | `autopilot_disabled_never_invokes_drive` | 同上 |
| 经 run+registry 启用并完成 | `autopilot_enabled_via_run_with_registry_completes` | 同上 |
| doom-loop guard 终止 ACT | `doom_loop_guard_terminates_act_phase` | 同上 |
| handoff 重置 transcript + 清 skill | `act_phase_handoff_resets_transcript_and_clears_skill` | 同上 |
| fallback 注入 execute_prompt | `act_phase_fallback_injects_execute_prompt_when_plan_has_no_text` | 同上 |
| 单元：parse_verdict 多变体 / should_stop | `parse_yes_variants` 等 9 项 | `session/src/autopilot/tests.rs` |

- 全量回归：`cargo test --workspace` → **1587 passed / 0 failed / 1 ignored**（当次实跑；ignored 为既有 `research_smoke_bing_wikipedia`，需真实 Chrome/网络）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
