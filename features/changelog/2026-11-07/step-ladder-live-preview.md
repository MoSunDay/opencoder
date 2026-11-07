# Step 阶梯：撤销自动披露，改为 turn 区实时预览（TUI + SPA）

一句话：阶梯回归**纯用户控制、默认折叠**（Say 是第一输出面，阶梯只是
drill-down）——删除全部自动开合机制（TUI `chat_step_live` 策略模块与
`user_touched` pin；SPA `steps/useLadderDisclosure.js` hook 与受控
activeKey/override）。运行中的 turn 改为在**最外层 turn 展示区**做实时
预览：折叠的 `N Steps` 组行下方直接流式渲染**当前（最后一个）step**
（Step(k) 行 + thinking + calls + 运行标记），settle（Say 首块 / Done /
Error / 新 turn）后预览消失，step 内容只存在于折叠的阶梯里；用户手动
展开的阶梯让位预览（内容不重复展示），用户状态全程不被流式事件移动。
本条取代 `2026-11-01/step-ladder-live-disclosure.md` 与
`2026-11-06/tui-live-*.md` 的自动披露口径。

## 行为口径

- 预览判定与 `progress_active` 生命周期不变（ToolStart / reasoning delta
  置位；Say 首块 / Done / Error / begin_turn / reanchor / reconcile /
  finish_bash_tool 复位），但语义从「驱动披露」变为「驱动渲染」。
- TUI 预览：`flatten_step_group` 折叠臂在组行后渲染最后一步的强制展开体
  （`push_step_body` 提取为共享渲染函数，阶梯循环与预览同一实现）；
  预览行**不产生点击目标**（折叠阶梯仍只有组行一个 target），行号镜像
  （`collect_headers`）新增 `preview_step_body_lines` 只计数不出 target；
  展开阶梯时预览让位（防重复）。
- TUI 物化/重绘门：thinking markdown 在 `(*open && step.open) ||
  should_show_progress` 时物化（预览需要）；`last_open_thinking_collapsed`
  把「折叠但运行中」视为可见，delta-only 帧不丢。
- TUI 删除面：`chat_step_live.rs` 整模块、`user_touched` 字段与全部
  pin 写点（toggle / Ctrl+L / `!cmd` 合成阶梯 / replay 合并 / 全部
  fixture）；用户 toggle 语义回归纯状态翻转。
- SPA 预览：`StepsContent` 用局部 `stepsOpen` useState 只控制 L0（默认
  折叠、跨流式 rerender 存活、epoch remount 重置），L1/L2/L3 回归
  非受控 Collapse；`ladderRunning && !stepsOpen` 时在 L0 下方渲染
  `StepPreview`（`Step(k)` 头 + `StepBody` 复用 + `ToolContent` 行，
  运行 tag/时长/错误就地流式），~12px 缩进 + 底部间距；删除
  `steps/useLadderDisclosure.js`（唯一消费方 stepsBlock.jsx）与
  `ToolContent` 的 `onUserExpand` pin。
- 运行中 call 的 spinner（TUI）/ running tag（SPA）保留；Ctrl+L 收起
  阶梯但预览是渲染层、运行中继续流式（TUI）；回放/快照阶梯永远无预览。

## 变更清单

TUI（`crates/tui`）：

- `src/chat_step_live.rs` — 删除（策略模块整体退役）。
- `src/chat_step_render.rs` — 折叠臂新增 live preview 渲染；step 体
  渐进提取为 `push_step_body`（阶梯/预览共用）；契约注释改写。
- `src/chat_headers.rs` — `collect_headers` 绑定 `progress_active`，
  折叠运行中分支走 `preview_step_body_lines`（只计数、无 target），
  `ends_on_expanded_call` 覆盖预览终态。
- `src/chat_steps.rs` / `chat_types.rs` — 删除 `step_live` 钩子与
  `user_touched` 字段；thinking 物化门扩展到预览；模块契约注释改写。
- `src/chat.rs` / `chat_helpers.rs` / `chat_steps_replay.rs` /
  `session_ui/replay.rs` — 删除 pin 写点与 mod 声明；toggle /
  collapse_all 注释更新。
- `src/chat_stream.rs` — `last_open_thinking_collapsed` 视折叠运行中
  为可见。

SPA（`crates/web/spa`）：

- `src/steps/useLadderDisclosure.js`、`src/steps/useLadderDisclosure.test.js`
  — 删除。
- `src/stepsBlock.jsx` — StepsContent 改局部 `stepsOpen`（仅 L0 受控），
  新增 `StepPreview` / `StepBody`（与阶梯共用），删除 ladder prop 线与
  `onUserExpand`；头部契约注释改写。
- `dist/` — `scripts/build-spa.sh` 重建，`check-spa-drift.sh` 零漂移。

文档：`agents/frontend/tui/index.md`、`agents/frontend/web/index.md`、
`repo-memory.md` 改为预览口径；旧 changelog（2026-11-01 / 2026-11-06
tui-live-*）头部加「已被本条取代」标记（正文保持历史原样）。

## 测试覆盖

TUI（`crates/tui/src/chat_tests/step_group/live_preview.rs`，原
`live_disclosure.rs` 重写更名）：

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行中 reasoning 在折叠组行下方预览 Step(1)+thinking | `running_reasoning_previews_the_last_step_below_the_folded_ladder` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| 运行中 call 预览（聚合行 + call 行 + spinner），阶梯保持折叠 | `running_calls_preview_with_spinner_and_the_ladder_stays_folded` | 同上 |
| 新 step 接管预览、旧 step 收回折叠阶梯 | `a_new_step_moves_the_preview_and_folds_the_previous_one` | 同上 |
| Say 首块 / Done 清除预览；Say 后新阶梯重新预览 | `say_first_chunk_and_run_end_clear_the_preview` | 同上 |
| Error 与 Done 同语义清除预览 | `error_clears_the_preview_like_done` | 同上 |
| 预览纯渲染：不新增点击目标；点组行即开阶梯 | `preview_rows_are_not_clickable_targets` | 同上 |
| 用户展开的阶梯让位预览且状态存活 settle | `user_opened_ladder_suppresses_the_preview_and_survives_the_settle` | 同上 |
| Ctrl+L 收起阶梯但预览继续流式 | `collapse_all_keeps_the_live_preview_visible` | 同上 |
| 回放阶梯无预览 | `replay_ladders_carry_no_preview` | 同上 |

TUI（既有测试改写）：

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 首 delta 物化 markdown（阶梯保持折叠、progress 置位） | `reasoning_delta_streams_into_the_live_preview_step` | `crates/tui/src/chat_tests/mod.rs` |
| 流式事件永不移动用户设定的层级 | `streaming_never_moves_user_set_levels` | `crates/tui/src/chat_tests/step_group/disclosure.rs` |
| 运行中（含用户折叠）delta 恒物化、Say settle 后入封存快照、Say 后新阶梯重新预览 | `live_preview_materializes_reasoning_while_the_turn_runs` | `crates/tui/src/chat_tests/thinking_state.rs` |
| 预览行数镜像对齐（thinking+展开输出+运行 call），无新 target | `live_preview_keeps_alignment_without_new_targets` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |
| 折叠运行中组（spinner 行）对齐：3 行 → 组行+预览+空行+marker | `step_group_running_call_keeps_alignment` | 同上 |
| 运行中边界恰好一个尾部空行（预览形态） | `tool_turn_boundary_keeps_exactly_one_trailing_blank` | `crates/tui/src/chat_tests/step_group.rs` |
| Say 后新阶梯组行行号落点（出生即折叠，无需 pin） | `step_row_line_index_lands_on_the_step_row` | `crates/tui/src/chat_tests/tool_collapse/turn_routing.rs` |
| 面板点击只翻转面板阶梯（出生折叠，点击=展开） | `sidecar_step_row_click_toggles_the_panel_view_not_the_main_transcript` | `crates/tui/src/app_helpers_tests/mouse_tests/hierarchy_and_actions.rs` |
| 预览可见期 delta-only 帧不合并 | `reasoning_deltas_render_first_frame_then_coalesce_hidden_updates` | `crates/tui/src/app_loop_bugfix_tests/streaming_and_clock.rs` |
| `!cmd` 合成阶梯 settle 后仍全展开（无 pin 断言） | `bash_tool_pin_survives_the_turn_end_settle`（原有用例，pin 断言瘦身） | `crates/tui/src/chat_tests/bash_tool.rs` |

SPA（`crates/web/spa/src/stepsBlock.dom.test.jsx`，live 组重写）：

| 功能 | 测试名 |
|------|--------|
| 运行中在折叠摘要下方预览最后一步 | `running turn previews the last step below the folded summary` |
| 预览随新 step 前移、阶梯保持折叠 | `preview advances to the newest step while the ladder stays folded` |
| 无 progress 字段走 open-call 回退预览 | `previews the running step via the open-call fallback without a progress flag` |
| settle 清除预览、阶梯保持折叠 | `settle clears the preview and the ladder stays folded` |
| 展开阶梯隐藏预览且跨流式 rerender 存活 | `opening the ladder hides the preview and the open state survives streaming rerenders` |
| 预览 call 行带 running tag（与组行 tag 并存，查询改 scoped） | `running tag appears on preview call rows, not on finished ones` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- `cargo test -p opencoder-tui --lib` → 1807 passed / 0 failed。
- `cargo clippy -p opencoder-tui --all-targets` → 0 warnings。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx src/chat.dom.test.jsx
  src/subagentBlock.dom.test.jsx`（46 passed）+
  `src/reduce.parity.test.js src/sayText.test.js
  src/steps/reducer.test.js`（44 passed）。
- `scripts/build-spa.sh` + `scripts/check-spa-drift.sh` → 零漂移。
