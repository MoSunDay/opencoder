# 实时区只渲染 thinking + calls：Say/阶梯计数样式完整回归（TUI + SPA）

> **位置已细化（TUI）**：本条「位置不变：TUI transcript 与 composer
> 之间的自有布局块」已被
> `2026-11-08/live-region-below-say-above-tok-cost.md` 取代 —— TUI
> 实时区改为 transcript **框内**底部条（Say 行下方、`[tok cost]` 边框
> 上方，`render_live.rs` 渲染），不再位于框外独立布局块；其余口径
> （内容、高度、settle）不变。SPA 落位维持本条描述。
>
> **内容已细化**：本条「实时区只渲染 thinking + call 行」中的 call 行
> 已被 `2026-11-08/live-region-calls-expanded-input.md` 细化 —— 实时区
> 调用行展开为 `❯ name` + 完整输入，结果一律收起（TUI 不渲染 output，
> SPA 收在默认关闭的 `result` 折叠）；「无 Step(k) 计数行」口径不变。

一句话：本条同时落定两件事。(1) **回退** `f6c97ef20`
（`2026-11-07/running-turn-only-in-live-region.md`）把运行中 turn 从
transcript/消息列表隐藏的口径：Say/阶梯转录区**逐字节**回到
`13875137f`（`2026-11-07/turn-live-region-own-area.md`）的计数样式
—— turn 运行中 transcript 照常保留折叠阶梯组行及其 running 提示
（TUI `▸ N Steps ⠋ running`，spinner 是组行上的 span 不是独立行，
`chat_flatten.rs`/`chat_headers.rs` 与 `13875137f` 完全一致；SPA
`StepsContent` 折叠 L0 组行带 running tag，`transcript.jsx` 无任何
运行中过滤），用户开合不被触碰；相邻 Say 时 running 提示并入
`Say(n steps)` 头行流转、Say 终态后退役。(2) **step 计数从此只归
阶梯**：turn 实时区只渲染当前 step 的明细行 —— thinking + call 行，
**无 Step(k) 计数行**（TUI `chat_step_render.rs` 把 `push_step_body`
拆成 `push_step_row`（计数行，阶梯循环专用）与 `push_step_details`
（明细行，阶梯与实时区共享），`chat_live_turn.rs` `render_lines` 只留
暗色 `⏺ live` 标记行 + `push_step_details`；SPA `TurnLiveRegion`
头部仅 `⏺ live`，主体 `StepBody` thinking + call 行）。位置不变：TUI
transcript 与 composer 之间的自有布局块；SPA transcript 滚动 div 与
QueuePanel 之间的固定面板。阶梯纯用户控制、撤销自动披露与 settle
生命周期口径继续有效。

## 行为口径

- TUI 转录区回归：`chat_flatten.rs`/`chat_headers.rs` 与 `13875137f`
  逐字一致 —— 运行中 `StepGroup` 照常折叠渲染组行（`▸ N Steps ⠋
  running`，两列间距）+ 尾随空行、照常产出组行 target，无任何
  `progress_active` 跳过逻辑；`Say(n steps)` 合并头、running 提示
  移交与退役语义原样回归。
- SPA 消息列表回归：`transcript.jsx` 与 `13875137f` 一致 —— 运行中
  turn 照常渲染 `StepsContent`（折叠 L0 组行 + running tag + 12px
  间距，L0–L3 全部非受控），无 `isStepsTurnRunning` 消息列表门控、
  无 `messageListItems` 过滤、无空气泡壳。
- TUI 实时区瘦身为明细：`chat_step_render.rs` `push_step_row`（开
  `› Step(k)` / 折 `▸ Step(k)` 计数行）只服务阶梯循环，
  `push_step_details`（thinking 块 + call 行 + 聚合 `N Function call`）
  成为阶梯展开体与实时区共享的同一实现；`chat_live_turn.rs`
  `render_lines` = 暗色 `⏺ live` 标记行 + 当前 step 的
  `push_step_details`（calls_open 强制开，运行 call 带 spinner span）
  —— 计数的唯一事实源是 transcript 阶梯（组行 `N Steps` / 展开后的
  `Step(k)` 行），实时区永不与之重复。
- SPA 实时区同口径：`stepsBlock.jsx` `TurnLiveRegion` 头部仅
  `⏺ live`（保留 error tag），主体 `StepBody`（thinking + 平铺
  call 行，running call 带 tag）；无 `Step(k)` 计数（计数归消息
  列表阶梯的 L0/L1）。位置不动：钉在 transcript 滚动 div 与
  QueuePanel 之间，绝不在气泡内。
- 渲染层纪律不变：实时区仍不入 flatten、零 target、零行数记账 ——
  滚动/选择/复制不受流式影响，运行中 call 头（spinner 字形）只在
  实时区行上、复制文本永不沾染；用户展开的阶梯与实时区**共存**，
  开合状态跨 settle 存活；Ctrl+L 收起全部可折叠层级、render-only
  的实时区照常流式；replay/snapshot 阶梯永不命中实时区。

## 功能 → 测试名

TUI 实时区语义（`crates/tui/src/chat_tests/step_group/live_preview.rs`）：

| 功能 | 测试名 |
|------|--------|
| transcript 保留折叠阶梯行 + running 提示；实时区流式 thinking 且无 `Step(` 计数行 | `running_reasoning_streams_in_the_turn_live_area` |
| 运行 call 在实时区流式（聚合计数 + call 行），阶梯保持折叠、组行自带提示 | `running_calls_stream_with_spinner_and_the_ladder_stays_folded` |
| 新 step 前移实时区（只留当前 step 明细），transcript 组行计数 `▸ 2 Steps` | `a_new_step_moves_the_live_area_and_folds_the_previous_one` |
| Say 首块 / run end 清空实时区、后续 turn 再武装 | `say_first_chunk_and_run_end_clear_the_live_area` |
| Error 与 Done 同样清空实时区 | `error_clears_the_live_area_like_done` |
| 实时区零 target（折叠阶梯仍只有组行一个 target） | `live_area_rows_are_not_clickable_targets` |
| 用户展开的阶梯与实时区共存（transcript 持阶梯行、实时区持明细），开合跨 settle 存活 | `user_opened_ladder_coexists_with_the_live_region_and_survives_the_settle` |
| Ctrl+L 收起全部层级、实时区继续流式（无计数行） | `collapse_all_keeps_the_live_region_visible` |
| 回放阶梯永不命中实时区 | `replay_ladders_carry_no_live_area` |

TUI 转录区边界、计数样式与复制（`chat_tests/step_group.rs`、
`chat_tests/tool_collapse.rs`、`copy_mode/clean_say_tests.rs`）：

| 功能 | 测试名 |
|------|--------|
| 运行中 transcript = 折叠组行 + 空行；实时区 = `⏺ live` + thinking + call 行（无 `Step(k)` 行）；settle 后并入 Say 头、单尾随空行 | `tool_turn_boundary_keeps_exactly_one_trailing_blank` |
| 折叠组行 running 提示（两列间距）、ToolEnd 保持、Say 首块移交合并头、Say 终态退役 | `running_hint_persists_until_say_begins` |
| 无 Say 的终态事件清 running（计数样式归属阶梯） | `terminal_event_clears_progress_when_no_say_arrives` |
| 默认折叠单组行（运行中照常计数） | `collapsed_by_default_renders_single_group_row` |
| 运行中 call 头（spinner 字形）只在实时区，复制文本永不沾染 | `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text` |

TUI 行数/对齐（`chat_tests/line_accounting/step_ladder.rs`）：

| 功能 | 测试名 |
|------|--------|
| 阶梯深度展开行数与 target 偏移（13875137f 样式回归基线） | `step_group_ladder_depth_alignment` |
| 展开 call 的行数与 target 偏移 | `step_group_with_expanded_call_keeps_alignment` |
| 运行中 call spinner 是组行 span 不是行、折叠组行 + 空行回归、记账不变式成立 | `step_group_running_call_keeps_alignment` |
| 用户展开阶梯的运行 step 行对齐（实时区不入 flatten、无计数行） | `auto_running_position_keeps_alignment` |
| 折叠态转录不含实时区行、零新增 target；实时区只出当前 step 明细、无 `Step(` | `live_region_stays_out_of_flatten_and_adds_no_targets` |

SPA（`crates/web/spa/src/stepsBlock.dom.test.jsx`，describe
`turn live region (its own area, outside the message list)`）：

| 功能 | 测试名 |
|------|--------|
| 运行 turn 最后一步流式在独立实时区（thinking + call 行、无 `Step(` 计数），消息列表保留折叠 L0 组行 + running tag；面板不在气泡内、文档序在摘要行之后 | `running turn streams the last step in the separate live region, not the transcript` |
| 实时区随新 step 前移、阶梯保持折叠 | `live region advances to the newest step while the ladder stays folded` |
| 无 progress 字段走 open-call 回退 | `streams the running step via the open-call fallback without a progress flag` |
| settle 清空实时区、阶梯保持折叠 | `settle clears the live region and the ladder stays folded` |
| 展开阶梯实时区仍在（阶梯自己的 `Step(2)` 行唯一）、开合跨流式 rerender 存活 | `opening the ladder keeps the live region and the open state survives streaming rerenders` |
| 实时区 call 行 running tag | `running tag appears on live region call rows, not on finished ones` |
| 平坦 tool turn 的 per-call running tag | ``flat `tool` turns share the same per-call running tag while open`` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` 过滤 `live_preview::`（9
  passed）与 `step_ladder::` +
  `tool_turn_boundary_keeps_exactly_one_trailing_blank` +
  `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text`
  + `running_hint_persists_until_say_begins` +
  `terminal_event_clears_progress_when_no_say_arrives` +
  `collapsed_by_default_renders_single_group_row`（10 passed）→ 19
  passed / 0 failed。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx` → 26 passed / 0
  failed。
