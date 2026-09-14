Commit: (working-tree, 基于 b465f440)

# 子代理中断后聚焦视图残留原始 Markdown 修复

用户反馈：Say 输出偶发在流结束后仍显示原始 Markdown（如 `**bold**`、`# 标题`）而非渲染结果。渲染契约是 `ChatBlock::Assistant { raw, rendered, done }`：`done: false` 时按 raw 逐行显示，`done: true` 后切换为渲染结果。症状等价于屏幕上残留一个未 finalize 的 Say。

排查路径：先排除父视图全链路——工作台 delta 丢弃由可靠的 `AssistantFinal` 修复；app.rs 渲染门控（dirty/render_pending/body_refresh_pending/skip_next_render + 333ms body ticker）经 4000 种随机交错仿真验证，任何可见事件后屏幕最多 333ms 收敛；随机多轮状态机 fuzz 与真实 worker 端到端测试均无法在父视图复现。

根因：子代理子视图（`SessionEvent::SubagentChild` 路由进 `ChatBlock::Subagent { view }` 的独立 ChatView）。子代理被取消（`SubagentEnd { cancelled }`）或父回合以 Error/Done 终止时由 `reconcile_orphaned_subagents` 孤儿修复，两条路径都只标记 Subagent 块状态、复位计时字段，从不调用子视图的 `finalize_assistant`。被中断的子运行不再产生自身的 `LlmRoundEnd`/`Done`，子视图的 Say 永远保持 `done: false`。用户聚焦该子代理（`[→ view]`）即看到持续显示的原始 Markdown，且该状态永久驻留（事后聚焦同样可见）。

修复：`mark_subagent_done`（crates/tui/src/chat.rs）与 `reconcile_orphaned_subagents`（crates/tui/src/chat_helpers.rs）在标记块完成时同步调用 `view.finalize_assistant()`。该函数幂等（per-block `done` 守卫），正常完成的子代理不受影响；嵌套子代理的孤儿修复同样受益。

## 测试覆盖

| 功能 | 测试或证据 |
| --- | --- |
| 取消的子代理子视图 Say 被 finalize | `chat::subagent_tests::cancelled_subagent_finalizes_child_say`（修复前失败） |
| 孤儿子代理（父 Done 时仍在流式）子视图 Say 被 finalize | `chat::subagent_tests::orphaned_subagent_child_say_finalized_on_done`（修复前失败） |
| 父视图渲染门控不残留过期帧（排除性验证） | `app::app_loop::tests::render_gate_fuzz::fuzz_last_frame_matches_final_state_after_turn_end`，4000 seeds |
| 父视图多轮随机状态机不残留 open Say（排除性验证） | `chat::say_raw_repro` |
| 真实 worker + MockChatClient 端到端渲染 | `chat::say_markdown_e2e`（evt 通道容量 4 强制丢弃压力） |

全量回归：`cargo test --workspace --lib --bins` 全绿（crates/tui 1709 项），`cargo fmt --check` 与 `cargo clippy --lib --tests` 无告警。

相关语义：[TUI](../../../agents/tui/index.md)。
