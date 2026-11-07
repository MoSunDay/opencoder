# Turn 实时区：独立于阶梯开合的流式渲染（TUI + SPA）

> **已取代**：本条的「TUI transcript 尾部尾缀 + SPA 阶梯下方兄弟节点」
> 位置口径已被 `2026-11-07/turn-live-region-own-area.md` 取代 —— turn
> 实时区是彻底独立的布局区（TUI 自有 Layout 块 / SPA 滚动区外固定
> 面板），不进 transcript 也不进气泡。本条保留为历史记录。

一句话：阶梯与实时展示是**两个区域**。阶梯回归**纯用户控制、默认折叠**
（Say 仍按原样把 steps 合并进 `Say(n steps)` 头）；运行中 turn 的当前
（最后一个）step 改在**独立的 turn 实时区**流式渲染 —— TUI 是
transcript 尾部的尾缀区（所有块之后），SPA 是阶梯下方的兄弟节点 ——
**从不**在 step 展开位置渲染、**从不**与阶梯开合状态耦合。settle
（Say 首块 / Done / Error / 新 turn）后实时区清空，完整 turn 折叠存于
阶梯；下一个 turn 立即运行时实时区直接显示它。本条取代
`2026-11-07/step-ladder-live-preview.md` 的「折叠组行下方预览 + 展开让
位」口径（该条的撤销自动披露部分继续有效）。

## 行为口径

- TUI 实时区：`flatten_with` 在全部块循环**之后**追加（`chat_live_turn.rs`
  新模块：`active_steps` 在当前 turn floor 内找 `progress_active: true`
  且非空的组 —— 每 turn 至多一个阶梯命中，replay 永不命中）；
  `flatten` 用共享的 `push_step_body` 强制展开渲染当前 step（Step(k) 行
  + thinking + calls + spinner）+ 一个尾随空行，无额外头、无点击目标。
- 零耦合：`flatten_step_group` 折叠臂只渲染组行 + 尾随空行；展开臂不受
  实时区影响。无任何 `!open` / `!stepsOpen` 门控 —— 展开阶梯与实时区
  **共存**（阶梯行照常渲染，实时区照常流式）。
- 行号镜像：`collect_headers` 不再为实时区计数/出 target（它是 transcript
  尾部，其后无块，hit-rect 无需对齐）；行数不变性测试在测试侧独立镜像
  尾区计数。
- `!cmd` 合成阶梯出生即 `progress_active: false`（完全展开的最终历史，
  实时区不重复它；call 行自身 spinner 仍显示活动）；`finish_bash_tool`
  的复位保留（no-op）。其余 `progress_active` 生命周期不变。
- SPA：L0 Collapse 完全非受控（删除 `useState stepsOpen` /
  activeKey/onChange）；实时区门控只剩 `ladderRunning && steps.length > 0`，
  展开阶梯不隐藏它。

## 功能 → 测试名

TUI（`crates/tui/src/chat_tests/step_group/live_preview.rs`，语义重写）：

| 功能 | 测试名 |
|------|--------|
| 运行中实时区流式渲染当前 step | `running_reasoning_streams_in_the_turn_live_area` |
| call 加入实时区带 spinner、阶梯保持折叠 | `running_calls_stream_with_spinner_and_the_ladder_stays_folded` |
| 新 step 接管实时区、旧 step 折叠入阶梯 | `a_new_step_moves_the_live_area_and_folds_the_previous_one` |
| Say 首块 / Done 清空实时区、后续 turn 再武装 | `say_first_chunk_and_run_end_clear_the_live_area` |
| Error 同 Done 清空实时区 | `error_clears_the_live_area_like_done` |
| 实时区不出点击目标 | `live_area_rows_are_not_clickable_targets` |
| 展开阶梯与实时区共存、开合状态跨 settle 存活 | `user_opened_ladder_coexists_with_the_live_area_and_survives_the_settle` |
| Ctrl+L 后实时区仍流式 | `collapse_all_keeps_the_live_area_visible` |
| replay 阶梯无实时区 | `replay_ladders_carry_no_live_area` |

TUI 行数/对齐（`chat_tests/line_accounting*`、`chat_tests/step_group.rs`）：

| 功能 | 测试名 |
|------|--------|
| 折叠块 + 尾区行数不变性（镜像含尾区计数） | `assert_line_accounting_matches` + 全部 line_accounting fixtures |
| 展开阶梯与尾区共存对齐（15 行） | `auto_running_position_keeps_alignment` |
| 折叠 + 尾区对齐且不增 target（14 行） | `live_area_keeps_alignment_without_new_targets` |
| 折叠块行 + 空行 + 尾区体 + 尾随空行 | `tool_turn_boundary_keeps_exactly_one_trailing_blank` |
| 合成 `!cmd` 阶梯出生即终态 | `push_bash_tool_starts_in_results_state` |

SPA（`crates/web/spa/src/stepsBlock.dom.test.jsx`，live 组重写）：

| 功能 | 测试名 |
|------|--------|
| 运行中折叠摘要下方实时区流式最后一步 | `running turn streams the last step in the live area below the folded summary` |
| 实时区随新 step 前移、阶梯保持折叠 | `live area advances to the newest step while the ladder stays folded` |
| 无 progress 字段走 open-call 回退 | `streams the running step via the open-call fallback without a progress flag` |
| settle 清空实时区、阶梯保持折叠 | `settle clears the live area and the ladder stays folded` |
| 展开阶梯实时区仍在（共存）且跨流式 rerender 存活 | `opening the ladder keeps the live area and the open state survives streaming rerenders` |
| 实时区 call 行带 running tag | `running tag appears on live area call rows, not on finished ones` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- `cargo test -p opencoder-tui --lib` → 1807 passed / 0 failed。
- `cargo clippy -p opencoder-tui --all-targets` → 0 warnings。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx`（26 passed）+
  `src/subagentBlock.dom.test.jsx`（7 passed）。
- `scripts/build-spa.sh` + `scripts/check-spa-drift.sh` → 零漂移。
