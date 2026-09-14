# 交错轮次滞留 open Say 致 `Say(n steps)` 正文永远原始 Markdown 修复

症状：`Say(n steps): xxx` 合并头下方的 Say 正文偶发永久以原始文本渲染（`**bold**`、`#` 等标记原样显示）。渲染契约：`ChatBlock::Assistant` 仅 `done:true` 走 markdown（`chat_flatten` done 分支），「不渲染」⇔ Say 永远停在 `done:false`。是否触发取决于 provider 是否在文本后继续吐思考（`think → text → think → text` 交错），故概率性复现。

根因（三点叠加）：

1. **一轮内可产生多个 open Say**：`append_text_delta` 只在「最后一个块是 open Assistant」时尾部续写；Say A 落地后 provider 再吐 `ReasoningDelta`，`append_step_thinking_delta` 在推进后的 turn floor（Say A 下方）插入 StepGroup，随后 TextDelta 发现尾部不是 Assistant → 新开 Say B。交错 K 次得 K 个 open Say。
2. **修复预算每次只修一个**：`finalize_assistant` 用 `rposition` 只 finalize 最后一个 open Say；回合末修复点共 3 个（`LlmRoundEnd` / `Done` / `TurnDone`）+ `AssistantFinal` 只修 anchor 指向的最后一个 → 最终轮 K≥4（或跨无工具轮累积）时更早的 Say 永久 `done=false`。
3. **两处边界完全不修**：`SidecarTurn` fold 从不 finalize 面板嵌套视图（且 runner 吞掉子 `Done`，面板内 K=2 即泄漏）；resume 重建 `build_subagent_block` 后子视图不补 finalize（live 路径 `mark_subagent_done` 会修）。

修复（crates/tui）：

- **A 根修** `chat_stream.rs::finalize_assistant`：`if let` 改 `while let` 循环——反复取 `rposition(done:false)` 并 finalize（markdown 渲染 + `context_used` 逐个入账），直到无 open Say。全部既有调用点（ToolStart/LlmRoundEnd/Done/TurnDone/push_marker/push_user/压缩/SubagentStart/mark_subagent_done/worker_dead 等）自动受益。
- **B 收紧不变量** `chat_stream.rs::append_text_delta`：push 新 Say 前（`flush_pending_thinking` → `seal_trailing_step` 之后）先 `finalize_assistant()` 封板旧 Say——有块介入后旧 Say 不可能再收到 delta，此时封板语义精确，轮内即转 markdown 渲染，K 被钉死为 1。
- **C 补漏边界**：`chat_sidecar.rs` `SidecarTurn` 臂设面板状态前 `panel.view.finalize_assistant()`（对齐 `mark_subagent_done`）；`session_ui/replay.rs` `build_subagent_block` 在 `reconstruct_child_view` 后对子视图补一次 finalize（事件日志截断在流式中途时不再悬空 open Say）。

## 测试覆盖

| 功能 | 测试或证据 |
| --- | --- |
| fuzz：每轮随机批次交错（含 think-after-text）+ 全部终态事件后无 open Say | `chat::tests::say_raw_repro::fuzz_say_finalized_after_turn_end`（扩展批次预算后修复前必红） |
| 最终轮 K=4 交错 → Done 后无 open Say、flatten 无 `**`、合并头/空行形状 | `chat::tests::say_interleaved_finalize::interleaved_k4_round_leaves_no_open_say_after_done`（修复前失败） |
| 交错 × 工具轮：每子轮计数 1/2/2/2/2 不累积、尾部 ladder 行、无原始标记 | `chat::tests::say_interleaved_finalize::interleaved_tool_round_seals_every_stranded_say`（形状钉死） |
| 新 Say 开启即封板旧 Say（K=1 不变量，轮内 markdown） | `chat::tests::say_interleaved_finalize::new_say_open_seals_the_previous_one_immediately`（修复前失败） |
| sidecar：子事件交错 + LlmRoundEnd + SidecarTurn → 嵌套视图无 open Assistant | `chat::tests::sidecar_fold::sidecar_turn_finalizes_interleaved_child_says`（修复前失败） |
| sidecar：流中途断（无 LlmRoundEnd）单 Say 也被封板 | `chat::tests::sidecar_fold::sidecar_turn_seals_a_single_stranded_child_say`（修复前失败） |
| resume：截断事件日志重建子视图 → 无 open Assistant、无原始标记 | `session_ui::subagent_block_tests::replay_truncated_child_log_seals_open_says`（修复前失败） |

回归：`cargo test --workspace` 除 `opencoder-worker::dag_live_logs`（30s 计时看门狗在重载机器上的既有环境性失败，stash 本次改动后同样失败，与本修复无关）外全绿；crates/tui 1698 项 lib 测试 + 全部集成测试通过；`cargo fmt --check` / `cargo clippy -p opencoder-tui --lib --tests` 干净。

相关语义：[TUI](../../../agents/tui/index.md)。
