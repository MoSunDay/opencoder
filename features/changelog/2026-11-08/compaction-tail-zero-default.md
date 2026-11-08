# Compaction 默认只保留摘要消息：tail_turns 默认 2→0，0 端到端合法

一句话：压缩（compaction）后的转录默认**只保留合成摘要消息** ——
`CompactionConfig.tail_turns` 默认从 2 改为 0，且 0 成为端到端完全
支持的取值：`preferred_split` 对 0 直接返回 `Some(messages.len())`
（整段转录进 head 摘要，单条/空转录仍是 "nothing to compact yet"），
`compaction_split` 在 `desired >= messages.len()` 时跳过
`boundary::tool_safe_split` —— 空尾之下所有 tool_use/tool_result 对
（含未应答的悬空调用）随 head 一起送摘要，配对约束平凡满足，无需
边界回溯；`split_index` 对 0 返回全长度（与理想口径一致）。`compact()`
不再 `.max(1)` 钳制。显式配置 `tail_turns >= 1` 保持原有「摘要 +
保留最近 N 轮」行为不变。

## 行为口径

- 默认（`tail_turns = 0`）：压缩后 `session.messages` 仅剩 1 条合成
  摘要消息（`[Conversation summary so far]` 前缀，synthetic）；
  `summary_seq` 记录整个被摘要 head 的软删除条数。
- `tail_turns = 0` 时拆分点恒为 `messages.len()`：不触发「轮数不足」
  回退，也不触发 tool 配对边界回溯（空尾无需为配对保留任何消息）。
- 单条消息（或空）转录在 0 下依旧无可摘要 → `Ok(None)` /
  "nothing to compact yet" 口径不变。
- 显式 `tail_turns >= 1`：理想轮感知拆分、超预算回退与
  `tool_safe_split` 边界保护全部照旧（旧行为）。
- serde 默认函数与 `Default` 实现同步为 0，config 文件缺省该键时
  与默认值一致。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| `tail_turns` 默认 0（Default 与 serde 缺省一致） | `compaction_tail_turns_defaults_to_zero` |
| `split_index` 尾长 0 返回全长度 | `split_index_zero_tail_returns_full_length` |
| 尾长 0 拆分覆盖全部转录（含已应答 tool 往返 + 悬空 tool_use，无边界回溯；单条/空转录仍为 None） | `compaction_split_zero_tail_summarizes_everything` |
| 默认配置端到端：压缩后仅剩摘要消息（synthetic、SUMMARY_PREFIX、`summary_seq=Some(4)` 全量 head 软删除） | `compact_default_keeps_only_summary` |
| 显式 `tail_turns = 2` 仍走保尾路径（u3 保留在尾部） | `compaction_fires_with_real_default_config` |
| 显式 `tail_turns = 1` 仍走保尾 + resume 尾部回读 | `compaction_persists_surviving_images_to_store` |
| 显式 `tail_turns = 2` 的 handoff OFFSET 算术（保尾形状）不变 | `clear_summary_prevents_offset_corruption` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- `cargo test -p opencoder-session --lib` → 505 passed / 0 failed
  （含 `compaction::tests::compact_default_keeps_only_summary` 等新增）。
- `cargo test -p opencoder-session --test compaction_and_model` /
  `--test compaction_persists_images` / `--test compaction_error_propagation` /
  `--test compact_none_over_budget` / `--test clear_context_compaction` /
  `--test handoff_clears_compaction` → 全部通过。
- `cargo test -p opencoder-core --lib config` → 139 passed / 0 failed。
- `cargo test -p opencoder-tui --lib worker` → 35 passed / 0 failed
  （`/compact` Done 契约在默认 0 下照常成立）。
- `cargo test -p opencoder-web --test web_api_ops compact` → 2 passed。
- `cargo clippy -p opencoder-core -p opencoder-session --all-targets -- -D warnings`
  → 0 warning。
