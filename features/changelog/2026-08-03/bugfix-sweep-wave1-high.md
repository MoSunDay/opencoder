Commit: (working-tree, pre-initial-commit)

# Bugfix Sweep Wave 1 — 7 High-Severity Defects

## 背景
全项目系统性 bug review（5 个并行子代理通读 8 个 crate ~84K 行）定位 ~50 个缺陷。本波次聚焦 7 个 High 级别：数据丢失、并发竞态、配置静默失效、OOM/DoS、状态泄漏。

## 变更

### H1 — LLM 完整回答被当作中断丢弃
- **`crates/llm/src/client.rs`**：`run_stream_once` 的 chunk-error 和 idle-timeout 两处返回 `Interrupted` 前，先判 `if finished`：若 `finish_reason` 已收到，emit `Completed` + `Ok(())`，避免完整回答被重试丢弃。新增测试 `run_stream_once_finalizes_when_finished_then_chunk_error`。

### H2 — Web drain 退出竞态：admitted prompt 永不处理
- **`crates/web/src/handle.rs`**：CAS-loss 分支（`swap` 返回 true）后新增 watchdog：轮询 `draining`（50ms × 100 = ~5s），一旦清零则查 `pending_inputs(Queue)`，有 pending 则竞态 spawn 新 drain。

### H3 — 三个配置段被 merge_into 静默丢弃
- **`crates/core/src/config/merge.rs`**：补 `output_streamline`（enabled/trim_trailing/collapse_blank_lines/trim_outer/collapse_inline_ws）、`tool_guard`（max_consecutive_failures/backoff_base_ms/backoff_max_ms）、`subagent_drain_secs` 三个分支。

### H4 — 压缩发生在 plan→act 交接后：状态泄漏
- **`crates/session/src/compaction.rs`**：`prev_skip` 改用 `session.summary_seq.or(session.handoff_seq).unwrap_or(0)`；SessionPatch 加 `clear_handoff: true`。
- **`crates/session/src/lib.rs`**：`after_compaction` 清 `handoff_seq`/`handoff_plan`。新增 3 个单元测试。
- **`crates/store/src/types.rs`**：`SessionPatch` 加 `clear_handoff: bool` 字段（`is_false` skip）。
- **`crates/store/src/libsql_store/sessions.rs`**：`update()` 处理 `clear_handoff` → `handoff_seq = NULL, handoff_plan = NULL`。

### H5 — read_bundle 无上限分配（OOM/DoS）
- **`crates/store/src/bundle.rs`**：分配前校验 `len <= 256 MiB`，超限 `bail!`。新增测试 `rejects_oversized_payload`。

### H6 — ProviderConfig::default() base_url 为空
- **`crates/core/src/config.rs`**：`Config::default()` 内 `provider.base_url = default_base_url()`（"https://api.openai.com/v1"），与 serde default 对齐。

### H7 — SSE 去重用 (kind,data) 内容 key → 事件丢失/重复
- **`crates/web/src/api.rs`**：改两级去重——有 seq 的事件用 `seq <= max_replay_seq` 精确比较（不碰撞）；无 seq 的事件保留内容指纹 fallback。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| H1: finished 后 chunk-error 不丢回答 | `run_stream_once_finalizes_when_finished_then_chunk_error` | `crates/llm/src/client.rs` |
| H3: 三配置段 merge 生效 | `merge_handles_output_streamline_tool_guard_subagent_drain` | `crates/core/tests/config_contract.rs` |
| H6: 默认 base_url | `default_provider_base_url_is_openai` | `crates/core/tests/config_contract.rs` |
| H4: 压缩清交接状态 | `compaction_after_handoff_clears_handoff_state` | `crates/session/src/lib.rs` |
| H4: prev_skip 优先级 | `prev_skip_zero_when_no_compaction_or_handoff`, `summary_seq_takes_priority_over_handoff_seq` | `crates/session/src/lib.rs` |
| H5: bundle 超限拒绝 | `rejects_oversized_payload` | `crates/store/src/bundle.rs` |
| H4: clear_handoff 写 NULL | `clear_handoff_nulls_handoff_fields`, `default_patch_leaves_handoff_fields_intact` | `crates/store/tests/clear_handoff.rs` |

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：所有文件 ≤800 行

## Impact Surface
- H1：所有 LLM 流式调用——连接抖动不再丢失已完成回答
- H2：Web 并发 prompt——退出窗口内 admit 的 prompt 不再静默丢失
- H3：配置文件中 `output_streamline`/`tool_guard`/`subagent_drain_secs` 现在生效
- H4：plan→act 交接后压缩不再算错 skip + 不泄漏交接状态
- H5：bundle 导入不再因损坏长度字段崩溃
- H6：无配置文件时默认 OpenAI base_url 正确
- H7：SSE 事件流不再因相同 payload 丢失/重复事件

## Related Docs
- [agents/session](../../agents/session/index.md)
- [agents/llm](../../agents/llm/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/store](../../agents/store/index.md)
- [agents/core](../../agents/core/index.md)
