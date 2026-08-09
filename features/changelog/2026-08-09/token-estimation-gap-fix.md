# fix(session,tui): 补齐 token 估算偏差——tool schema + queue/steer 计入 context

## 背景

本地 token 估算（驱动 compaction 预算 + TUI ctx% 指示器）与 provider 返回的真实
`prompt_tokens` 之间存在系统性偏差，导致两类问题：

1. **Tool-definition JSON 未计入预算**：provider 的 `prompt_tokens` 始终包含完整的
   tool schema 数组（act agent 约 340 tokens），但本地估算完全忽略了这块成本。
   compaction 因此滞后触发，TUI ctx% 也系统性偏低。

2. **排队/转向消息未计入 context_used**：`QueueConsumed` / `SteerConsumed` 事件
   在 transcript 中被渲染为 `ChatBlock::User`，但其 token 成本从未累加到
   `ChatView::context_used`。导致 ctx% 指示器少报每个排队/转向 prompt 的全部 token，
   是"显示 70k 但 128k 就触发 compaction"困惑的主要来源。

## 变更

### `crates/session/src/tools/mod.rs` — 新增 `estimate_tool_schema_tokens`
- 新增 pub fn `estimate_tool_schema_tokens`：镜像真实 LLM 调用中的 tool 过滤逻辑
  （agent allowlist ∧ latent-gating），构建同样的 OpenAI function-calling JSON schema，
  序列化后通过 `opencoder_llm::estimate` 估算 token 数。
- 新增 2 个内联 unit test。

### `crates/session/src/compaction.rs` — 预算纳入 tool schema tokens
- `estimated_tokens` 在 `base`（messages + system prompt）之上再
  `saturating_add(tool_tokens)`，使 compaction 预算与 provider 的 `prompt_tokens` 对齐。

### `crates/tui/src/chat.rs` — track_context 计入 queue/steer
- `track_context` 新增 `QueueConsumed` / `SteerConsumed` 两个 match arm，
  各自 `context_used += estimate(text)`，补齐此前缺失的用户 prompt token。

### `crates/tui/src/app_helpers.rs` — sys_tokens_for 纳入 tool schema
- `sys_tokens_for` 返回值加入 `estimate_tool_schema_tokens(...)`，
  使 TUI 系统提示基线与 compaction 预算保持一致。

### `crates/tui/src/render_status.rs` — CONTEXT_BASELINE 归零
- `CONTEXT_BASELINE` 从原值降为 `0`。tool schema tokens 已纳入 `used`，
  不再需要减去基线，ctx% 直接显示真实消耗百分比。

### `crates/session/tests/compaction_and_model.rs` — 既有测试适配新核算
- `reserved_budget_actually_shrinks_usable_window` 和
  `global_agents_md_counts_toward_compaction_budget` 的 message 大小 / context_limit
  参数上调，使 token 总量在新核算（含 tool schema ~340 tokens）下仍能清晰区分
  reserved 边界两侧的行为。断言语义不变。

### `crates/tui/src/chat_tests/mod.rs` — 新增 queue/steer context_used 测试
- 2 个纯同步 unit test 验证 `context_used` 在收到 QueueConsumed/SteerConsumed 后确实增长。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Tool schema token 估算非平凡（act） | `estimate_tool_schema_tokens_is_nontrivial` | `crates/session/src/tools/mod.rs` |
| Tool schema token 估算（plan） | `estimate_tool_schema_tokens_plan_excludes_build_hint` | `crates/session/src/tools/mod.rs` |
| QueueConsumed 计入 context_used | `ctx_counts_queue_consumed_prompt` | `crates/tui/src/chat_tests/mod.rs` |
| SteerConsumed 计入 context_used | `ctx_counts_steer_consumed_prompt` | `crates/tui/src/chat_tests/mod.rs` |
| reserved 收缩可用窗口（适配新核算） | `reserved_budget_actually_shrinks_usable_window` | `crates/session/tests/compaction_and_model.rs` |
| 全局 AGENTS.md 计入预算（适配新核算） | `global_agents_md_counts_toward_compaction_budget` | `crates/session/tests/compaction_and_model.rs` |

- 全量回归：`cargo test --workspace -- --test-threads=2` → **2291 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：compaction.rs 799 行（≤ 800）；chat.rs 791 行（≤ 800）；其余均 < 400

## 备注

- `--test-threads=2`：`bash_normal_completion` / `bash_failure_appends_exit_code`
  在默认高并发下偶发失败（fork 子进程在资源受限环境下超时，exit code -1），
  降线程数后稳定通过。与本次变更无关。
