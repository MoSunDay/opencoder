# 压缩摘要展示顺序修正（TUI）

一句话：压缩折叠后重建的视图把 `Compaction` 摘要块渲染在最顶部，视觉顺序
变成「摘要 → 保留的最近交互 → 继续执行」，读起来像摘要插错了位置。现在改为
「保留的最近交互 → 摘要块 → 继续执行」：摘要块标记的是折叠发生的位置，
应落在保留尾部与续跑之间。

## 变更清单

- `crates/tui/src/session_ui/replay.rs` — 回放入口（`replay_into_chat` /
  `replay_messages`）在消息循环里拦截压缩摘要合成消息，收集后于常规块
  与 step 折叠完成时经 `push_compaction_summary_block` 追加到视图末尾；
  `replay_one` 不再就地渲染摘要块（其余合成消息仍跳过）。
- 工作区内他人未提交的存储原型（后被整段回退、未合入）— 顺手修复其
  无法编译的问题：`_tmp` 改为 `Option<Arc<TempDir>>` 使 `#[derive(Clone)]`
  成立（仅解除编译阻塞，不改动原型设计）。

## 行为口径

- LLM 上下文顺序不变：折叠后的消息列表仍是 `[summary, tail...]`，
  `runner/llm_call.rs` 按原序发送（对模型而言摘要先于尾部才是时间序正确）。
  本次只改 UI 展示顺序，两层顺序自此解耦。
- 直播路径不变：`CompactionDelta` 流式块仍在对话末尾展开；`TranscriptReset`
  重建后摘要块落在保留尾部之后、在途回显（`rebuild_after_reset` 的 echo
  重推）之前；随后 `Compaction(summary)` 事件按既有幂等去重逻辑落位，
  不重复堆块。
- Web SPA 本就按事件到达顺序在末尾追加 `🗜` 行，无需改动。

## 测试覆盖

> 测试重命名映射：`replay_renders_summary_head_as_collapsed_compaction_block`
> （2026-11-02）→ `replay_defers_summary_block_below_retained_tail`（本次，
> 断言顺序随行为翻转）。历史 changelog 保持原样。

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 摘要块延迟到保留尾部之后渲染 | `replay_defers_summary_block_below_retained_tail` | `crates/tui/src/session_ui/compaction_replay_tests.rs` |
| 摘要不污染 `first_prompt` | `replay_summary_does_not_pollute_first_prompt` | `crates/tui/src/session_ui/compaction_replay_tests.rs` |
| resume 回放渲染摘要块（含顺序断言） | `resume_replay_renders_summary_head` | `crates/tui/tests/compaction_summary_replay.rs` |
| reset 重建 + 事件幂等 + tail/摘要/续跑顺序 | `transcript_reset_then_compaction_event_renders_single_block` | `crates/tui/tests/compaction_summary_replay.rs` |

- 定向回归：`cargo test -p opencoder-tui --lib compaction` → 25 passed / 0 failed；
  `cargo test -p opencoder-tui --test compaction_summary_replay` → 2 passed / 0 failed
- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings
  -A clippy::large_enum_variant -A dead_code -A clippy::result_large_err` →
  零警告。三项 allow 均为工作区既有问题（core/fleet `NodeOperation`、
  store 未完成的存储原型、session `result_large_err`），与本次改动无关；
  `crates/tui` 内无任何告警。
- 测试范围：本次修改及其直接影响的链路（TUI 压缩回放）；未运行全量测试。
