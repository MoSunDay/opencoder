# 统一压缩回放顺序：Compaction 块在摘要持久化位置就地渲染

一句话：`/task` 切换与 resume 的回放入口此前把压缩摘要块延迟收集、
在其余块（含续跑）之后统一压栈，导致 Compaction 标记排在转录末尾；而
执行中 `TranscriptReset` 重建按 `[summary]` 头部回放、续跑内容追加在
下，Compaction 渲染在最前——同一份持久化转录在两个入口呈现两种顺序。
现在两条路径统一为就地渲染：`Compaction` 块落在摘要消息的持久化位置
（折叠转录头部），视图顺序恒为 `[Compaction, 保留尾段, 续跑]`，与压缩
完成时执行路径所见一致；发给 LLM 的 `[summary, tail...]` 消息列表顺序
（`runner/llm_call.rs` 原样发送）不变。

改动面：`crates/tui/src/session_ui/replay.rs`（`replay_into_chat_inner`
与 `replay_messages` 删除 `deferred_summaries` 延迟收集，改为消息循环内
`push_compaction_summary_block` 就地压栈，`continue` 语义不变），单测与
集成测试同步翻转顺序断言。沿革：2026-11-06 的
`tui-compaction-summary-order.md` 引入「延迟到尾部之后」的 UI 顺序，
本次因与执行时 `TranscriptReset` 重建顺序不一致而回正为头部就地渲染；
该历史 changelog 按其自述保持原样。仅 TUI 回放展示顺序，不触及规则
04/06/07 契约，无公共能力/CLI/API 变更，不触发 Ontology 同步；未运行
全量测试。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 回放就地渲染 summary 块 | `replay_renders_summary_block_at_transcript_head` | `crates/tui/src/session_ui/compaction_replay_tests.rs` |
| resume 顺序 | `resume_replay_renders_summary_head` | `crates/tui/tests/compaction_summary_replay.rs` |
| mid-run 保留尾段顺序 | `transcript_reset_keeps_prior_tail_turns_but_drops_in_flight_echo` | `crates/tui/tests/compaction_summary_replay.rs` |
| /task 切换与执行时同序 | `task_switch_replay_matches_execution_order` | `crates/tui/tests/compaction_summary_replay.rs` |

- 定向回归（全部通过，未运行全量测试）：
  - `cargo test -p opencoder-tui session_ui::compaction_replay_tests` → lib 单测 4 passed / 0 failed（含翻转后的 `replay_renders_summary_block_at_transcript_head`）
  - `cargo test -p opencoder-tui --test compaction_summary_replay` → 7 passed / 0 failed（含新增 `task_switch_replay_matches_execution_order` 与两处翻转断言）
  - `cargo test -p opencoder-tui --test session_switch_restore` → 3 passed / 0 failed（回归，未改动）
  - `cargo test -p opencoder-tui --test tok_cost_replay` → 8 passed / 0 failed（相邻回放消费方，确认无顺序依赖破坏）
- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。
