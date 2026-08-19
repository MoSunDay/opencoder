Commit: (working-tree, post-3320cbb)

# 修复 compaction 误杀：清空上下文后 "found nothing to summarize" 报错

## Context

用户报告：清空 context 时有概率抛出
`compaction failed: transcript exceeds context window but compaction found
nothing to summarize`，会话被 Err 打断。

根因：`/act_clear_context`（保留计划）与 plan→act 交接把 transcript 折叠成
**单条 synthetic 消息**，但 `last_usage`（模型上报的 input_tokens）仍是折叠前
长会话的旧值。下一轮 `should_compact` 用旧 reported usage 触发（`>= budget`），
`compaction_split` 对单条消息返回 `None`（无头可摘要），`run_loop` 的
`Ok(None)` 分支无条件 `Err` 杀会话——即使当前 transcript 极小、请求完全能发。

另一触发形态：单条巨消息自身估算就超 compaction 预算（预算只是阈值，不是
硬上限），此前同样被误杀。

## Change Summary

- **`crates/session/src/lib.rs`**：`after_handoff` / `after_compaction` 重置
  `last_usage = Usage::default()`——transcript 折叠后旧上报用量失去意义，
  从根上消除 stale-reported 触发。
- **`crates/session/src/runner/mod.rs`**：`Ok(None)` 分支不再无条件杀会话。
  当前 transcript 估算（`compaction::estimated_tokens`）低于 provider 硬
  context limit 时直接继续发 LLM 请求（compaction 预算≠硬上限）；仅在估算
  超过硬 limit（请求必然 400、且无头可摘要）时才保留原有 Err + Error 事件。
- **`crates/session/src/compaction/mod.rs`**：`estimated_tokens` 改
  `pub(crate)` 供 runner 硬限制门使用（doc 注明语义）。

## 测试清单（功能 → 测试名）

| 功能 | 测试 |
| --- | --- |
| 清空上下文 + 陈旧上报用量仍执行计划 | `crates/session/tests/clear_context_compaction.rs::clear_context_with_stale_usage_still_executes_plan` |
| 超预算但低于硬上限 → 继续发请求（原误杀回归） | `crates/session/tests/compact_none_over_budget.rs::over_budget_but_under_hard_limit_proceeds_to_llm_call` |
| 超硬上限 + 无可摘要 → 保留 Err 防护 | `crates/session/tests/compact_none_over_budget.rs::over_hard_limit_with_nothing_to_compact_errors_before_llm_call` |
| 折叠点重置 reported usage | `crates/session/src/compaction/tests.rs::transcript_collapse_resets_reported_usage` |

## Validation

- `cargo fmt --all --check` 干净
- `cargo test -p opencoder-session`：全绿（详见实跑输出）
- 既有回归套件（plan 交接、compaction 系列）未受影响
