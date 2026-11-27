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

- `crates/tui/src/session_ui/reset_fold.rs`（初版落在 `replay.rs`，收尾时按 800 行预算迁出）：
  `rebuild_view_from_messages(chat, msgs)`——
  `rebuild_after_reset` 的 store-free 镜像：`[摘要, 尾段, 续跑]` 同序折叠；结转
  `tokens_total`（取 max，`[tok cost]` 不回退）、`annotation_text` / `submitted` /
  `first_prompt`（保留旧值，与父路径同规则）、`subagents_running` / `subagents_total` 与孙代理
  `Subagent` 卡片一并结转——卡片按「task id == 派生消息里 `task` tool_use 的 id」重挂在派生它的
  assistant 消息之后（与 `replay_into_chat` 的 `tasks_by_parent` 同位；卡片 token 已在结转的
  `tokens_total` 内，重挂不再累加），派生消息已被折叠的卡片按 orphan 规则追加在尾段之后并保持
  可路由——父视图离线重建对这类记录是直接丢弃，嵌套折叠手里有活卡片，选择保留；
  `pending_turn_echo` 与父视图同规则——压缩重建丢弃、
  clear-context 重建保留；`turn_block_start` 推到重建后的块尾（可靠补写不得命中 reset 前的块）。
- `crates/tui/src/chat.rs`：`SessionEvent::TranscriptReset(msgs)` 分支由空操作改为调用上述
  重建。父视图的 reset 在 `app_loop.rs` 被拦截（离线重建），不会走到这里；只有嵌套视图
  （subagent 子视图经 `SubagentChild` 路由、sidecar 面板视图经 `SidecarChild` 路由）或
  `reconstruct_child_view` 事件回放进入。
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
| 嵌套 reset 重挂孙代理卡片（派生消息存活）、计数一致、事件仍可路由 | `child_live_reset_reattaches_grandchild_cards_under_their_parent_message` | `crates/tui/src/session_ui/reset_fold_tests.rs` |
| 派生消息已折叠的孙代理卡片按 orphan 追加、不丢弃 | `child_live_reset_keeps_grandchild_cards_whose_spawning_message_was_folded` | 同上 |
| 事件回放路径：`SubagentStart` 卡片跨 `TranscriptReset` 存活 | `child_replay_keeps_grandchild_cards_across_the_folded_reset` | 同上 |
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
- 收尾记录（评审后）：`first_prompt` 结转由「旧值缺省回退回放值」改为与父路径完全同规则
  （直接保留旧值；无 store 回放下兜底值只会是保留尾段里的 prompt，并非首条 prompt，且嵌套
  视图无 `first_prompt` 消费方）；`store_free_rebuild_carries_lifetime_state_but_drops_compacting_echo`
  改为携带 `Some` 首条 prompt 断言，`store_free_rebuild_keeps_echo_when_reset_is_not_a_compaction`
  增加「尾段 prompt 不被提升为 first_prompt」断言；`chat.rs` 分支注释补上 remote 父路径为 app loop
  内联回放。复跑 `--lib subagent_compaction`（5）、`--lib transcript_reset`（9）、`--lib tok_cost`（22）、
  `--lib sidecar`（50，含 `SidecarChild` 路由进同一分支）、`--lib first_prompt`（3）；clippy 零警告、fmt 无差异。
- 收尾记录（P2 修复）：孙代理卡片随嵌套折叠结转（见上）；`rebuild_view_from_messages` 迁至
  `session_ui/reset_fold.rs`（`replay.rs` 787 → 738 行），`replay_messages` 改为其无卡片参数的薄封装；
  新增 `reset_fold_tests.rs` 3 例（共享 `subagent_compaction_tests` 的夹具）。共享工作区当时含他人
  在途且暂时无法编译的测试文件，故在「HEAD + 本次改动」的隔离 worktree 复跑：`--lib reset_fold`（4，
  含 1 例既有同名匹配）、`subagent_compaction`（5）、`compaction`（30）、`subagent`（79）、
  `transcript_reset`（9）、`tok_cost`（22）、`sidecar`（50）、`replay`（44）、`reset_rebuild`（2）、
  `first_prompt`（3）；`--test compaction_summary_replay`（7）、`resume_context_used`（7）、
  `tok_cost_replay`（8）；均 0 failed；clippy 零警告、fmt 无差异。
- 收尾记录（P3 假设核实，仅读码确认、无代码改动）：
  1. sidecar 面板不会随嵌套 reset 丢失。`rebuild_view_from_messages` 的 `*chat =` 只作用于嵌套视图，而
     `sidecar_focus` 置 true 仅三处：`chat_sidecar.rs` 的 Start 分支（门禁要求它已为 true，不会把 false 翻成
     true）、`app.rs` 的 `SidecarAsk` 与 `sidecar_ui::enter_panel`（二者由 app loop / slash 派发传入顶层
     `chat`）。嵌套视图的 `sidecar_focus` 恒为 false、`sidecar` 恒为 `None`：`SidecarStart` 在
     `fold_sidecar` 的 Start 门禁处被吞掉，`SidecarChild` 查不到同 id 面板也被吞掉；渲染
     （`app_display.rs` 的 `sidecar::focused`）与鼠标折叠（`collapse_view`）同样只读顶层视图。sidecar 面板
     自身的嵌套视图虽经 `SidecarChild` 进入同一重建分支，但面板状态（`done/ok/answer/total_tokens/
     rounds/elapsed_ms`）与焦点都在外层 `SidecarPanel` / 顶层视图上，不受 `*chat =` 影响。
  2. 压缩不会留下「`ToolResult` 在尾段、派生它的 `task` `ToolUse` 消息被折叠」。`compact` 的切点来自
     `compaction_split`：尾段非空时经 `boundary::tool_safe_split` 回溯，接受切点的条件是每个 tool_use
     要么整体落在尾段（`start >= split`），要么其 tool_result 严格落在被摘要的头部（`end < split`）；
     未应答的 `task` tool_use 因此必然连同派生消息一起留在尾段（卡片走重挂，而非 orphan 追加）。
     `tail_turns == 0` 时尾段为空，整对随头部一起折叠；clear-context 的 reset 把消息列表折成单条边界
     标记（`fold_to_continuity_seed`），同样不会残留 ToolResult。前提是 tool_use id 在转录内唯一——
     `tool_safe_split` 对重复 id 取最后一次出现，属既有边界逻辑，本次未改。
  本轮仅文档改动，只核对了内容与引用路径；未重跑测试（上一轮隔离树结果见上）。
- 测试范围：本次修改及其直接影响链路；未运行全量 Rust / SPA 测试与浏览器验收。

## Impact Surface

- TUI：进入 subagent 视角时，压缩后的子转录以 `Compaction` 摘要块开头，压缩前历史与历次
  旧摘要不再显示；父视图、Web、CLI/API、依赖与 Ontology 目录均未改变。
- 孙代理卡片（收尾修复，初版为已知局限）：子视图压缩后其孙代理 `Subagent` 卡片不再消失——重挂在
  派生消息之下，派生消息已折叠的按 orphan 追加；`subagents_running/total` 与可见卡片保持一致，
  运行中的孙代理事件继续路由进卡片。父视图离线重建仍走 store 记录，行为不变。
