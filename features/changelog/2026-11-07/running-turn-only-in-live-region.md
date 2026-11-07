# 运行中 turn 只在实时区：settle 前完全不进 Steps 折叠与消息列表（TUI + SPA）

一句话：运行中 turn（Say 未开始，`progress_active` 仍置位）**只**渲染在
独立 turn 实时区 —— TUI 的 transcript **完全不包含**它：无
`▸ N Steps ⠋ running` 折叠组行、无尾随空行、无 hit target
（`chat_flatten.rs` `flatten_with` 跳过 `progress_active: true` 的
`StepGroup`，零 transcript 行；`chat_headers.rs` 同步跳过，hit-rect 对齐
不漂移）；SPA 的消息列表同样不渲染它（共享判定 `isStepsTurnRunning`
同时门控消息列表与实时区，`messageListItems` 过滤运行中条目 —— 无空
气泡壳）。settle（Say 首块 / run end / 新 turn）后折叠阶梯**原样**回到
transcript / 消息列表（相邻时并入 `Say(n steps)` 单头行），用户开合与
行数对齐全程不变。本条细化 `2026-11-07/turn-live-region-own-area.md`
的「实时区是运行 turn 的流式面」口径：该条下运行 turn 仍在 transcript
留一条折叠组行，本条把这最后一行也收进实时区；`progress_active` 是
唯一判定源，阶梯纯用户控制、撤销自动披露与 settle 生命周期口径继续
有效。

## 行为口径

- TUI flatten 跳过：`chat_flatten.rs` `flatten_with` 对 `progress_active:
  true` 的 `StepGroup` 直接跳过 —— 运行中组贡献**零** transcript 行
  （无折叠组行、无尾随空行、无 hit target），相邻块的单空行纪律只对
  已 settle 的组生效；运行中 turn 的唯一展示面是实时区
  （`chat_live_turn.rs` + `render.rs` 自有布局块）。
- TUI headers 同步跳过：`chat_headers.rs` 以同一门控跳过运行中组（零
  target、零行数记账），与 `flatten_with` 逐字对齐 —— hit-rect 与行号
  镜像永不漂移；运行中 call 头（spinner 字形）只出现在实时区，复制
  文本永不沾染（沿上一条口径）。
- settle 回归原状：`progress_active` 翻 false（Say 首块 / run end / 新
  turn）后，折叠阶梯以既有形态回到 transcript —— 相邻 Say 时并入
  `Say(n steps)` 单头行（running 提示随头行流转、Say 终态后退役），
  用户展开的阶梯照常渲染每一行，开合状态跨 settle 存活；replay /
  snapshot 阶梯永不命中实时区（沿上一条口径）。
- SPA 共享判定：`stepsBlock.jsx` 导出 `isStepsTurnRunning(turn)` ——
  有 Say 即已 settle 返回 false；否则 `progressActive === true`（老式
  手搭 turn 无该字段时回退 open-call 判定），同时接受 `steps` turn 与
  `assistant_turn` 气泡内容；`findRunningStepsTurn` 从最新 turn 往回
  扫描该判定。
- SPA 双面同门：`transcript.jsx` `AssistantTurnContent` 运行中不渲染
  `StepsContent`，防御性独立 `steps` role 同门控；`messageListItems()`
  过滤运行中 steps turn 条目 —— 消息列表无 L0 组行、无阶梯内容、无
  空气泡壳；钉在 transcript 滚动 div 与 QueuePanel 之间的
  `TurnLiveRegion` 是运行中 turn 的唯一展示面。settle 后折叠阶梯回到
  消息列表，开合纯用户控制。

## 功能 → 测试名

TUI 主语义（`crates/tui/src/chat_tests/tool_collapse.rs`、
`chat_tests/step_group/live_preview.rs`）：

| 功能 | 测试名 |
|------|--------|
| 运行中 transcript 零行（无组行/空行/target），实时区唯一展示面；Say 首块 settle 后组行并入 Say 头、running 提示随头流转且 Say 终态退役 | `running_turn_lives_only_in_the_live_region_until_say_begins` |
| 运行中无阶梯可开（展开状态也不产生行/target）；settle 后折叠阶梯回归并入 Say 头，用户开合存活 rerender | `no_ladder_to_open_while_running_settled_ladder_opens_and_survives` |
| Say 首块 / run end 清空实时区、后续 turn 再武装 | `say_first_chunk_and_run_end_clear_the_live_area` |
| Ctrl+L 后实时区仍流式（运行中本就无阶梯行） | `collapse_all_keeps_the_live_region_visible` |

TUI 边界与复制（`chat_tests/step_group.rs`、`copy_mode/clean_say_tests.rs`）：

| 功能 | 测试名 |
|------|--------|
| 运行中转录零行 + 实时区强展开当前 step；settle 后 Say 头 + 单个尾随空行 | `tool_turn_boundary_keeps_exactly_one_trailing_blank` |
| 运行中 call 头只在实时区、复制文本永不沾 spinner 字形 | `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text` |

TUI 行数/对齐（`chat_tests/line_accounting/step_ladder.rs`）：

| 功能 | 测试名 |
|------|--------|
| 运行中 call 只在实时区流式（spinner 是 span 不是行）、零 transcript 行零 target、记账不变式成立 | `step_group_running_call_keeps_alignment` |
| 运行中用户展开的阶梯同样零 transcript 行（零 target）；settle 后同一开阶梯逐行渲染、hit-rect 对齐 | `running_open_ladder_keeps_alignment` |
| 运行中组零 transcript 行、零新增 target，实时区流式当前 step | `live_region_stays_out_of_flatten_and_adds_no_targets` |

TUI 相邻面（`chat_tests/terminal_safety.rs`、`chat_tests/thinking_state.rs`、
`chat_tests/sidecar_stream_isolation.rs`、`chat_tests/tool_collapse/turn_routing.rs`）：

| 功能 | 测试名 |
|------|--------|
| 终端安全 sanitize 同时覆盖 transcript 与实时区（transcript 永不携带实时区行） | `thinking_delta_is_sanitized_once_before_it_reaches_rendering` |
| 运行中每个 reasoning delta 都物化（实时区可见），settle 后只存于折叠阶梯 | `live_preview_materializes_reasoning_while_the_turn_runs` |
| 面板运行中阶梯不出现在面板 transcript（实时区唯一展示面），settle 后回到 echo 问题下方 | `panel_ladder_lands_below_the_echoed_question` |
| 运行中组无 target；settle 后 header_line_idx 落在组行 | `step_row_line_index_lands_on_the_step_row` |

SPA（`crates/web/spa/src/stepsBlock.dom.test.jsx`，describe
`turn live region (its own area, outside the message list)`）：

| 功能 | 测试名 |
|------|--------|
| 运行 turn 只在实时区流式 —— 消息列表无 L0 组行/阶梯内容/空气泡壳 | `running turn streams ONLY in the live region — the message list shows nothing of its ladder` |
| 实时区随新 step 前移、消息列表仍无组行 | `live region advances to the newest step; the message list still shows nothing` |
| 无 progress 字段走 open-call 回退（同一判定驱动双面） | `streams the running step via the open-call fallback without a progress flag` |
| settle rerender 后折叠阶梯回到消息列表、实时区清空 | `settle rerender returns the folded ladder to the message list and clears the region` |
| settle 后阶梯开合纯用户控制、状态跨 rerender 存活（实时区不再回来） | `after settle the ladder opens/closes on pure user control and the state survives rerenders` |
| 实时区 call 行 running tag（唯一 running tag） | `running tag appears on live region call rows, not on finished ones` |
| 平坦 tool turn 的 per-call running tag | ``flat `tool` turns share the same per-call running tag while open`` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` 过滤本条映射的 13 个测试名 →
  13 passed / 0 failed。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx` → 26 passed / 0
  failed。
