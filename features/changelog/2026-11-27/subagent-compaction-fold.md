# Subagent 子视图的 Compaction 展示与父转录同序：一个摘要块、位于转录头部

一句话：subagent 子视图收到 `TranscriptReset` 后不再把压缩摘要追加在完整历史尾部（多次压缩
还会叠出多个 `Compaction` 块），而是像父视图的离线重建一样就地折叠消息列表——
`[Compaction, 保留尾段, 续跑]`，一个折叠摘要块恒在子转录最顶部。

## 背景与决策

- 用户反馈：进入 subagent 视角看到的 Compaction 有 2 个且不在最顶部；父 agent 的展示是
  「一个 Compaction 在转录头部」，子视图应当一致。
- 根因（两条路径同源）：
  1. 实时路径：runner 在子会话压缩后依次发出 `CompactionDelta…` → `TranscriptReset(msgs)` →
     `Compaction(summary)`，经 `SubagentChild` 路由进子视图；`ChatView::apply` 对
     `TranscriptReset` 是空操作（父视图由 app loop 拦截后走离线重建），于是流式摘要块留在
     原地、压缩前历史一行不少，`finalize_compaction` 只能去重「孪生」而不折叠历史。
  2. 回放路径：`reconstruct_child_view` 从子会话持久化事件逐条 `apply`，同样不折叠；子会话
     每压缩一次就多一个摘要块，全部按时间顺序散落在完整历史中。
- 决策：`ChatView::apply(TranscriptReset(msgs))` 改为**无 store 的同步重建**
  （`session_ui::rebuild_view_from_messages`），复用 `replay_messages` 的折叠与
  `rebuild_after_reset` 的跨 reset 结转规则。父视图不受影响：其 reset 仍由 app loop 拦截并
  走带 store（子任务记录 + 图片预取）的离线重建，永远不会进入该分支。

## 行为与实现

- `crates/tui/src/session_ui/replay.rs`：新增 `rebuild_view_from_messages(chat, msgs)`——
  `rebuild_after_reset` 的 store-free 镜像：`[摘要, 尾段, 续跑]` 同序折叠；结转
  `tokens_total`（取 max，`[tok cost]` 不回退）、`annotation_text` / `submitted` /
  `first_prompt`（保留旧值，缺省回退回放值）、`subagents_running` / `subagents_total`
  （孙代理计数；其块不经记录重建）；`pending_turn_echo` 与父视图同规则——压缩重建丢弃、
  clear-context 重建保留；`turn_block_start` 推到重建后的块尾（可靠补写不得命中 reset 前的块）。
- `crates/tui/src/chat.rs`：`SessionEvent::TranscriptReset(msgs)` 分支由空操作改为调用上述
  重建。父视图的 reset 在 `app_loop.rs` 被拦截（离线重建），不会走到这里；只有嵌套视图
  （subagent 子视图）经 `SubagentChild` 路由或 `reconstruct_child_view` 事件回放进入。
- 结果：子视图压缩后只剩**一个**折叠 `Compaction` 块且位于 `blocks[0]`；连续压缩两次时
  第二次 `TranscriptReset` 携带的折叠转录只含最新摘要，旧摘要块随历史一起消失——与父视图
  resume / `/task` 切换 / mid-run 重建所见完全一致。
- `real_context_tokens` 契约随之统一：嵌套 reset 现在就是一次 replay 重建，provider truth
  来自存活消息的 usage（空 reset 得 `None`），不再保留 reset 前的陈旧值。

## 功能 → 测试映射

| 行为 | 测试名 | 位置 |
|------|--------|------|
| 子事件回放：历史折叠为一个头部摘要块 | `child_replay_folds_history_into_one_head_summary` | `crates/tui/src/session_ui/subagent_compaction_tests.rs` |
| 子事件回放：两次压缩只剩最新摘要 | `child_replay_two_compactions_keep_only_the_latest_summary` | 同上 |
| 实时 `SubagentChild(TranscriptReset)` 路由同序折叠 | `child_live_reset_folds_the_nested_view_like_the_parent` | 同上 |
| 无 store 重建的跨 reset 结转（tok cost / 孙代理计数 / echo） | `store_free_rebuild_carries_lifetime_state_but_drops_compacting_echo` | 同上 |
| clear-context reset 保留 echo、无 Compaction 块 | `store_free_rebuild_keeps_echo_when_reset_is_not_a_compaction` | `crates/tui/src/session_ui/subagent_compaction_tests.rs` |
| reset 重建经 replay 重算 provider truth | `real_context_survives_compaction_transcript_reset_and_model_switch` | `crates/tui/src/chat_tests/tok_cost.rs`（断言更新） |

- 定向回归：`cargo test -p opencoder-tui --lib subagent_compaction` → 5 passed；
  `--lib compaction` → 30 passed；`--lib subagent` → 87 passed；`--lib transcript_reset` → 9 passed；`--lib tok_cost` → 22 passed；
  `--lib replay` → 43 passed；`--lib reset_rebuild` → 2 passed；`--lib timer` → 16 passed；
  `--lib sidecar` → 50 passed；`--test compaction_summary_replay` → 7 passed；
  `--test resume_context_used` → 7 passed；`--test tok_cost_replay` → 8 passed；均 0 failed。
- Clippy：`cargo clippy -p opencoder-tui --all-targets` → 零警告；`cargo fmt --check` 干净。
- 收尾隔离验证：把暂存内容应用到 `HEAD` 的独立 worktree（排除共享工作区中无关的在途 hunk）
  复跑 `--lib subagent_compaction`（5）、`--lib compaction`（30）、`--lib subagent`（79）、
  `--lib transcript_reset`（9）、`--lib tok_cost`（22）全部通过，`cargo fmt --check` 无差异，
  clippy 零警告——提交内容自身可独立构建、不依赖未提交的他人改动。
- 测试范围：本次修改及其直接影响链路；未运行全量 Rust / SPA 测试与浏览器验收。

## Impact Surface

- TUI：进入 subagent 视角时，压缩后的子转录以 `Compaction` 摘要块开头，压缩前历史与历次
  旧摘要不再显示；父视图、Web、CLI/API、依赖与 Ontology 目录均未改变。
