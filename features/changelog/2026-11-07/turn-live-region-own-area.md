# Turn 实时区：自有布局区，不进 transcript 尾部也不进 steps 气泡（TUI + SPA）

一句话：运行中 turn 的当前（最后一个）step 流式渲染在一个与 Say/Step
消息区**完全分离的独立 UI 区域** —— TUI 是 transcript 主体与
queue/composer 之间的**自有布局块**，SPA 是 transcript 滚动 div 与
QueuePanel 之间的**固定面板** —— 既**不**追加到 transcript 尾部，也
**不**进入/挂在 steps 气泡内部或下方。滚动、选择、复制与行号镜像
全部不受流式影响；阶梯折叠/展开纯用户控制，settle（Say 首块 / Done /
Error / 新 turn）后实时区清空，完整 turn 折叠存于阶梯。本条取代
`2026-11-07/turn-live-area-independent.md`（`3b10d934a`，TUI transcript
尾部尾缀 + SPA 阶梯下方兄弟节点的位置口径）与更早
`2026-11-07/step-ladder-live-preview.md`（`5afb497be`，与阶梯开合耦合的
折叠组行下方预览）；两条中的阶梯纯用户控制、撤销自动披露与
`progress_active` 生命周期口径继续有效。

## 行为口径

- TUI 自有布局块：`render.rs` 的 Layout 在 transcript 主体与
  queue/composer 之间切出自有 chunk；`chat_live_turn.rs` 提供
  `active_steps(blocks, turn_start)`（当前 turn floor 内找
  `progress_active: true` 且非空的组 —— 每 turn 至多一个阶梯命中，
  replay 永不命中）与 `render_lines(steps, anim_tick)`（暗色 `⏺ live`
  头 + 用共享 `push_step_body` 强制展开的当前 step 体），由 `render.rs`
  以 Paragraph 渲染；高度上限 `height/3` 且保尾部（最新行优先），
  `plan_active` 时整区归零关闭。
- `chat_flatten.rs` **永不**产出 live 行：transcript 只含 Say + 阶梯块，
  滚动计数、复制/选择提取、hit-rect 与行号镜像全部与流式无关；运行中
  call 头（spinner 字形）只出现在实时区，复制文本永不沾染。
- 零耦合不变：无任何 `!open` / `!stepsOpen` 门控 —— 用户展开的阶梯与
  实时区**共存**；Ctrl+L 折叠所有可折叠层级，render-only 的实时区照常
  流式；replay / snapshot 阶梯永不命中实时区。
- SPA：`stepsBlock.jsx` 导出 `TurnLiveRegion({turns})`（带
  `data-turn-live-region` 的边框面板，头 `⏺ live · Step(k)`）与
  `findRunningStepsTurn(turns)`（从最新 turn 往回找仍在运行的 steps
  turn；最新 steps turn 已有 Say 即已 settle 返回 null，无
  `progressActive` 字段回退 open-call 判定）；`chat.jsx` 把
  `<TurnLiveRegion turns={stream.turns} />` 固定渲染在 transcript 滚动
  div 与 QueuePanel 之间 —— 永远不在消息列表 / 气泡内。
- settle 语义不变：Say 首块 / Done / Error / 新 turn 清空实时区，完整
  turn 折叠存于阶梯；下一个 turn 立即运行时实时区直接显示它的当前
  step。

## 功能 → 测试名

TUI 实时区语义（`crates/tui/src/chat_tests/step_group/live_preview.rs`）：

| 功能 | 测试名 |
|------|--------|
| 运行中实时区流式当前 step；flatten 永不产出 live 行（transcript 不沾实时区行） | `running_reasoning_streams_in_the_turn_live_area` |
| call 加入实时区带 spinner、阶梯保持折叠 | `running_calls_stream_with_spinner_and_the_ladder_stays_folded` |
| 新 step 接管实时区、旧 step 折叠入阶梯 | `a_new_step_moves_the_live_area_and_folds_the_previous_one` |
| Say 首块 / Done 清空实时区、后续 turn 再武装 | `say_first_chunk_and_run_end_clear_the_live_area` |
| Error 同 Done 清空实时区 | `error_clears_the_live_area_like_done` |
| 实时区 render-only 无点击目标 | `live_area_rows_are_not_clickable_targets` |
| 展开阶梯与实时区共存、开合状态跨 settle 存活 | `user_opened_ladder_coexists_with_the_live_region_and_survives_the_settle` |
| Ctrl+L 后实时区仍流式 | `collapse_all_keeps_the_live_region_visible` |
| replay 阶梯无实时区 | `replay_ladders_carry_no_live_area` |

TUI 边界与复制（`chat_tests/step_group.rs`、`copy_mode/clean_say_tests.rs`）：

| 功能 | 测试名 |
|------|--------|
| 运行中转录只剩折叠阶梯行 + 单个尾随空行（当前 step 在实时区） | `tool_turn_boundary_keeps_exactly_one_trailing_blank` |
| 运行中 call 头只在实时区、复制文本永不沾 spinner 字形 | `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text` |

TUI 行数/对齐（`chat_tests/line_accounting/step_ladder.rs`）：

| 功能 | 测试名 |
|------|--------|
| 阶梯各层深度行数不变性（hit-rect 对齐） | `step_group_ladder_depth_alignment` |
| 展开 call 的行数与 target 偏移 | `step_group_with_expanded_call_keeps_alignment` |
| 运行中 call spinner 是 span 不是行、组行对齐不变 | `step_group_running_call_keeps_alignment` |
| 用户展开阶梯的运行 step 行对齐（实时区不入 flatten） | `auto_running_position_keeps_alignment` |
| 折叠态转录不含实时区行、零新增 target | `live_region_stays_out_of_flatten_and_adds_no_targets` |

SPA（`crates/web/spa/src/stepsBlock.dom.test.jsx`，describe
`turn live region (its own area, outside the message list)`）：

| 功能 | 测试名 |
|------|--------|
| 运行 turn 最后一步流式在独立实时区（不在气泡内、文档序在摘要行之后） | `running turn streams the last step in the separate live region, not the transcript` |
| 实时区随新 step 前移、阶梯保持折叠 | `live region advances to the newest step while the ladder stays folded` |
| 无 progress 字段走 open-call 回退 | `streams the running step via the open-call fallback without a progress flag` |
| settle 清空实时区、阶梯保持折叠 | `settle clears the live region and the ladder stays folded` |
| 展开阶梯实时区仍在（共存）且跨流式 rerender 存活 | `opening the ladder keeps the live region and the open state survives streaming rerenders` |
| 实时区 call 行 running tag | `running tag appears on live region call rows, not on finished ones` |
| 平坦 tool turn 的 per-call running tag | ``flat `tool` turns share the same per-call running tag while open`` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` 过滤 live_preview（9
  passed）、line_accounting（14 passed）、
  `tool_turn_boundary_keeps_exactly_one_trailing_blank` 与
  `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text`
  （各 1 passed），全部 0 failed。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx` → 26 passed / 0
  failed。
