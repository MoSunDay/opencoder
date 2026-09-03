# /sidecar 面板体感三修：无教程、阶梯在问题之下、展开后 thinking 持续流式

Commit: 5410f6d（源码随同车提交落地；测试与文档由后续提交补齐）

## Context

用户实测 `/sidecar` 报告两个不符预期：

1. 进入面板就看到新手教程（欢迎提示）；提交问题后 `N Steps` 计数行出现在 User 回显**上方**。
2. 展开某个 Step 能看到 thinking 流式，但随后流式"停止"，看不到新的 thinking 输出。

## 根因

- **教程泄漏**：`render_body` 的空会话教程门是 `is_top_level && blocks.is_empty() && !submitted`，而 `is_top_level` 只看 `subagent_focus.is_none()`。sidecar 聚焦时 body 换成面板的全新空 `ChatView`（submitted=false），被当成顶层空会话渲染了教程。
- **阶梯在回显之上**：`echo_question` 把 User 块推进 `panel.view` 后没有重锚 `turn_block_start`（保持默认 0），首个 ReasoningDelta 在 floor 0 处 INSERT StepGroup——插到 User 之前。主 transcript 的 queue 消费回显早有 `reanchor_turn_after_user_echo`（5410f6d 引入），sidecar 回显漏接。
- **展开后流式冻结**：`append_step_thinking_delta` 的追加分支 `if open && step.open { render_step_thinking(step) } else { thinking_dirty = true }`——展开时 `toggle_tool_call_at` 已经 render 过一次并清了 dirty，后续 delta 走 open 分支时 `render_step_thinking` 因 `thinking_dirty == false` 直接跳过：raw 一直在涨、屏幕永远停在展开瞬间的快照。
- **伴生**：`collapse_view`（body 点击路由）不认识 `sidecar_focus`，面板内点 step/call 行会去 toggle 主 transcript 同索引块——面板内阶梯根本无法展开（或静默翻转主视图同索引组）。

## Change Summary（crates/tui）

- `app_loop.rs::body_is_top_level(chat, subagent_focus)`：`subagent_focus.is_none() && !chat.sidecar_focus`；`app.rs` 渲染参数改用该助手——聚焦 sidecar 面板不再是"顶层"，空面板体不再渲染教程（与子代理子视图同规则）。
- `sidecar_ui.rs::echo_question`：回显落块后调用 `panel.view.reanchor_turn_after_user_echo()`——面板回合的阶梯 floor 锚在问题之下，`N Steps` 永远渲染在 User 之后（含追问回显）。
- `chat_steps.rs::append_step_thinking_delta`：追加分支先无条件置 `thinking_dirty = true` 再按可见性渲染——展开中的 Step 每个 delta 都重渲染，流式不再冻结；折叠时仍只 O(1) 追加 raw。
- `app_mouse.rs::collapse_view`：`sidecar_focus` 时路由到 `chat.sidecar` 的嵌套 view——面板体内 step/call/thinking 行的点击（hit-rect 块索引本就是面板相对）落到面板视图，主 transcript 同索引块不受牵连。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| 展开后同一段 thinking 的后续 delta 持续渲染且保序 | `chat_tests/step_group.rs::expanded_step_keeps_streaming_new_thinking_deltas` |
| 面板阶梯块序与扁平序都在回显问题之下 | `chat_tests/sidecar_stream_isolation.rs::panel_ladder_lands_below_the_echoed_question` |
| 聚焦 sidecar 非顶层（教程门关闭）、子代理同否 | `app_loop_tests/sidecar_display_tests.rs::focused_sidecar_body_is_not_top_level` |
| 面板内点 group 行开面板阶梯、主视图同索引组不动 | `app_helpers_tests/mouse_tests/hierarchy_and_actions.rs::sidecar_step_row_click_toggles_the_panel_view_not_the_main_transcript` |

四例均在回退对应 hunk 后转红（源码临时还原验证）。

## 回归

- `cargo test -p opencoder-tui --lib`：1636 passed / 0 failed。
- `cargo test --workspace` + clippy + fmt：本轮按用户指示跳过，待下轮补跑。
- SPA 无代码变更（sidecar 为 TUI 专属；SPA 阶梯 reducer 本就直写状态无 dirty 门）。

## Related Docs

- [每个用户输入一个阶梯](turn-boundary-per-user-input.md)（`reanchor_turn_after_user_echo` 的主 transcript 侧）
- [tui 模块](../../../agents/tui/index.md)
