# Step 阶梯流式自动披露（TUI + SPA）

一句话：运行中的 turn 不再整体藏进 `▸ N Steps` 单行——当前 running step
（最后一个未完成 step）在阶梯运行位自动展开，thinking 增量与 call 行实时
渲染、运行中的 call 行带 spinner/running 标记；step 完成（下一次 Thinking
开新 step）或 turn 结束（Say 首块 / Done / Error / 新 turn）自动折回
`N Steps` 摘要。用户手动点开/折叠任一层后，自动机制本 turn 内不再碰该层。
回放/快照路径保持全折叠不变。

## 变更清单

TUI（`crates/tui`）：

- `src/chat_types.rs` — `StepGroup`/`Step` 增加 `user_touched`；披露契约
  文档更新（「streaming 永不改披露」→「运行位自动、用户优先」）。
- `src/chat_step_live.rs`（新增）— 纯函数披露策略：`activate`（运行位
  自动展开）/ `settle`（收起）/ `settle_steps_before_last`（新 step 折旧），
  全部跳过 `user_touched` 的层。
- `src/chat_steps.rs` — `set_turn_progress(true/false)` 分别驱动
  activate/settle（ToolStart/Say 首块/Done/Error/begin_turn/reanchor 五类
  调用点自动生效）；`append_step_thinking_delta`/`append_step_call` 新 step
  诞生时折旧 step 并自动打开新 step；全部构造点补 `user_touched`。
- `src/chat.rs` — `toggle_tool_call_at` 各 target 置对应 `user_touched`；
  `collapse_all_collapsible`（Ctrl+L）置 `user_touched` 抑制流式重开。
- `src/chat_helpers.rs` — `!cmd` 阶梯按用户展开固定（`user_touched: true`）。
- `src/chat_step_render.rs` — 运行中的 call（`elapsed_ms.is_none()`）header
  行追加 spinner（同行追加，行数口径不变）。
- `src/copy_mode/clean.rs` + `src/chat_types.rs`（`SPINNER` 放宽为
  `pub(crate)`）— 复制模式剥离运行中 call header 行尾的 spinner span
  （字形属 chrome，header 文本保留）。
- 零改动收益：`visible_targets`/`collect_headers`/鼠标命中由 flag 驱动自动
  对齐；`last_open_thinking_collapsed` 重绘优化在用户折叠后仍生效。

SPA（`crates/web/spa`，server Rust/SSE 零改动）：

- `src/steps/useLadderDisclosure.js`（新增）— override 感知的受控披露 hook
  + 纯函数 `autoOpenKeys(running, stepCount)`；最终 open = override ?? auto。
- `src/stepsBlock.jsx` — L0/running step/calls 聚合改为受控自动展开；step
  完成自动折旧；Say/结束整体收起；运行中 call 加 `running` Tag；L3 单条
  call 行保持不受控，但用户展开任一行会经 `onUserExpand` 通知把其
  `step:N`+`calls:N` 钉住（override=true），settle 后钻取结果不再被收起
  （TUI Call 行点击置 `user_touched` 的 SPA 对齐）。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行位自动展开（thinking 实时可见） | `live_reasoning_auto_opens_the_running_position` | `crates/tui/src/chat_tests/step_group/live_disclosure.rs` |
| 工具活动保持运行位展开 | `tool_activity_keeps_the_running_position_open` | 同上 |
| 新 step 折旧、运行位接替 | `a_new_step_folds_the_finished_one_and_takes_over` | 同上 |
| Say 首块与 Done/Error 收起、Say 后新阶梯重新自动展开 | `say_first_chunk_and_run_end_settle_the_ladder`、`error_settles_the_ladder_like_done` | 同上 |
| 用户折叠/点开优先于自动机制 | `user_collapse_of_an_auto_opened_level_wins_for_the_turn`、`user_opened_levels_survive_the_settle` | 同上 |
| Ctrl+L 抑制流式重开 | `collapse_all_pins_levels_against_the_stream` | 同上 |
| 回放保持全折叠 | `replay_ladders_stay_fully_collapsed` | 同上 |
| 披露策略纯函数（开/收/跳过 user 层/目标定位） | `activate_opens_group_and_last_step`、`activate_respects_user_touched_levels`、`settle_collapses_auto_levels_but_keeps_user_levels`、`settle_before_last_skips_user_steps_and_short_lists`、`canonical_group_targets_first_group_after_floor` | `crates/tui/src/chat_step_live.rs`（内联） |
| 用户优先（既有语义修订） | `new_output_never_closes_user_opened_ladder_levels` | `crates/tui/src/chat_tests/step_group/disclosure.rs` |
| `!cmd` 固定展开在 turn 结束收起后保持 | `bash_tool_pin_survives_the_turn_end_settle` | `crates/tui/src/chat_tests/bash_tool.rs` |
| 运行中 call 行 spinner 不进复制文本（复制模式剥离） | `running_call_header_cleans_to_its_label_only` | `crates/tui/src/copy_mode/clean_say_tests.rs` |
| 既有钻取/折叠/鼠标/行数断言按新契约修订 | `tool_call_expand.rs`、`tool_collapse*.rs`、`step_group.rs`、`thinking_state.rs`、`say_pair_dedup.rs`、`terminal_safety.rs`、`tool_output_blank.rs`、`hierarchy_and_actions.rs`、`streaming_and_clock.rs`、`render_clear_tests.rs`、`chat_tests/mod.rs` | `crates/tui/src/**` |
| SPA auto key 纯函数 | `useLadderDisclosure.test.js`（5 例） | `crates/web/spa/src/steps/` |
| SPA 自动展开/收起/用户优先/running 标记（DOM） | `live ladder disclosure (auto-open + user override)` 组（原 8 例 + 下行新增钉住用例；sayActive 折叠用例在更早的 `StepsContent three-level drill-down` 组）+ 既有钻取用例修订 | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| SPA 展开单条 call 行钉住其 step 与 calls 聚合跨 settle（TUI Call 行 `user_touched` 对齐） | `an expanded call row pins its step and calls open across the settle` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |

## 定向回归

- `cargo test -p opencoder-tui --lib` → 1801 通过（含新增 live_disclosure 9 例
  与 chat_step_live 内联 5 例）。
- `cargo clippy -p opencoder-tui --all-targets` → 0 警告。
- `cd crates/web/spa && npx vitest run src/stepsBlock.dom.test.jsx
  src/steps/useLadderDisclosure.test.js src/steps/reducer.test.js
  src/bubbleItems.test.js src/reduce.parity.test.js src/saypairs.e2e.test.js`
  → 88/88 通过（另加影响链 `subagentBlock.dom.test.jsx`、`chat.dom.test.jsx`
  → 20/20）。

## 配置与依赖

- 无新增依赖；无协议/持久化变化（披露 flag 为纯 UI 态，不落盘）。

## 回滚步骤

- 还原上述 TUI/SPA 文件即回滚；`user_touched` 字段随枚举还原一并消失，
  无数据迁移。

## 遗留

- Web 的 Ctrl+L/⤒ 收起是 epoch 重挂载：turn 仍在跑时 auto 会重新生效
  （TUI 因 `user_touched` 不重开）——已在 `stepsBlock.jsx` 头注释写明，
  属可接受差异。
- `TextDelta("")` 字面空块在生产解码层已被过滤（`crates/llm` 两处
  `!t.is_empty()` 门禁），不会触发「空 Say 不收起」路径。

## 工程拆分（纯重构，行为不变）

- 回放侧阶梯归一（`coalesce_steps` / `normalize_turn_groups` /
  `take_rendered_thinking`）自 `chat_steps.rs` 移至
  `chat_steps_replay.rs`，为后续迭代留出行数余量；调用路径经
  `chat::coalesce_steps` 重导出保持不变。
- 覆盖不变：沿用 `chat_tests/step_group/replay.rs` 既有用例（如
  `replay_merges_adjacent_tool_rounds_into_one_group`、
  `replay_pairs_each_ladder_with_its_own_say`、
  `replay_folds_thinking_without_a_following_group_into_the_ladder`）——
  原 `chat_steps.rs` 内联单测仅覆盖留在原处的函数，无需随迁。
