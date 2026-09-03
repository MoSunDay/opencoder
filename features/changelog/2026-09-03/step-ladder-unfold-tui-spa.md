# TUI+SPA 工具调用 step 阶梯恒展开重构

Commit: (working-tree, step 阶梯去组级折叠，TUI 与 Web SPA 同步对齐)

## 背景

- 用户否定组级折叠：旧模型整个 `StepGroup` 默认收成一行 `▸ N steps`（可点击开合），全部 step 被这一行吞掉，不点开看不到任何步。两个伴生混乱：步内 thinking 旧标签 `❯ Say:` 与终答头撞名（用户感知「Say 被收了」）；步收起时 standalone `💭 Thinking` 行残留（「thinking 收起也不全」）。
- 目标模型一句话：`n × step（thinking + n call（执行+结果））+ Say`——组标静态不可折叠、step 行恒可见、步/call 二级展开、Ctrl+L 一键收齐、Say 永不折叠。TUI 与 Web SPA 同步对齐。

## 变更

- TUI 侧（`crates/tui/src/`）：`chat_types.rs` `ChatBlock::StepGroup` 删 `open` 字段（组级开合态不复存在）；`chat.rs` 删 `toggle_step_group_at`，`collapse_all_collapsible` 只收步/call 两级；`render_hits.rs` + `app_mouse.rs` 删组行鼠标命中（`ToolBtn` 仅剩步/call 行）；`chat_step_render.rs` `flatten_step_group` 恒渲染静态标行 `≡ N steps`（不可点击/不可折叠，任一 call 未完附 `⠋ running` spinner）+ 每步行 `❯ Step(n)`/`▸ Step(n)`，步内 thinking 头改标 `💭 Thinking`；`chat_headers.rs` 行数核算镜像二级模型；`copy_mode/clean.rs` copy 文本匹配 `≡ N steps` 并去掉 `💭 Thinking` 头部 chrome；`chat_helpers.rs` `!cmd` 的 `finish_bash_tool` 收折步与 call 展开（非组）。终答 `❯ Say:` 顶层块不变。
- SPA 侧（`crates/web/spa/src/`）：`reduce.js` 把非 task 工具调用折叠为 `{kind:'steps',steps:[{thinking,calls:[…]}]}` turn——live 路径镜像 TUI `chat_steps.rs` 边界启发（尾随 think run 弹入步内、新 call 在尾步无 finished call 时并入、孤儿 tool_end 合成 finished call），`task` 工具保持旧扁平行 + 🤖 subagent 块；快照 `turnsFromMessages` 改消息对语义（一条 assistant message 的 tool_use×N = 一步，相邻组合并，reasoning buffer 作 pending thinking）；新增 `stepsBlock.jsx` 渲染阶梯（`ThinkContent`/`ToolContent` 自 `transcript.jsx` 迁入）：静态 `≡ N steps` 标行（antd Tag 标 running/error）+ 每步一个 `❯ Step(k)` Collapse + 单 call 二级展开；`transcript.jsx` 加 `BUBBLE_ROLES.steps` + 一键收起（window Ctrl/Cmd+L keydown 与 `⤒ 收起` 链接 bump epoch key 重挂 Bubble.List → remount 复位全部 Collapse）；`subagentBlock.jsx` childLines 把 steps turn 压平为每 call 一行。
- `task`(subagent) 工具行为两侧均不变（SPA 扁平行 + 🤖 块、TUI Subagent 块）。
- 已知差异：TUI replay 快照仍按 per-call step（born-finished，一步一 call）；SPA 快照按消息对一步一 message——多 call 单消息重载后 SPA 合并为一步。live 语义两侧一致，仅持久化重载路径的分组粒度不同，记录为已知差异。

## 测试清单

- TUI `cargo test -p opencoder-tui`：1698 passed。新增 `multi_round_turn_renders_marker_steps_and_answer_without_any_click`（`crates/tui/src/chat_tests/step_group.rs`，零点击可见 marker+步行+终答）；重写 `step_group.rs`/`ctrl_l_tests.rs`/`mouse_tests.rs`/`tool_call_expand.rs`/`tool_collapse.rs`/`line_accounting.rs`/`bash_tool.rs`/`sidecar_stream_isolation`/`copy_mode/clean.rs` 断言对齐新模型（组行无点击目标、collapse 只两级）。
- SPA `npm test`：129 passed / 14 文件。新增 `stepsBlock.dom.test.jsx` 6 用例（marker+步行动零点击可见、running/error 标签、步→call 二级下钻断言 input/output、Ctrl+L 与 `⤒ 收起` 收齐、Say 独立气泡）；`reduce.test.js` 新增 (a)–(i) 步折叠用例 + 更新 3 处旧扁平断言 + subagent 子事件 steps 形状；`subagentBlock.dom.test.jsx` +1 childLines steps；`bubbleItems.test.js` +1 steps 角色。

## 回归

- `cargo test --workspace`：3954 passed / 0 failed。
- `npm run build`：dist 重建（`include_bytes!` 编译期内嵌，dist 不重建则 SPA 变更不生效）。
