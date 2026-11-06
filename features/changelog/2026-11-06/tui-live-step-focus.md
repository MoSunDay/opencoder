# 直播态 Step 阶梯只渲染运行中的 step（TUI）

一句话：同一 turn 内多次工具往返时，流式自动披露会在运行位把**每个**
历史 step 行都展开，长 turn 直接把视口刷成一列 `Step(i)`。现在运行位只
渲染最新的一个 step（thinking + 本轮 calls），历史 step 行折叠进组行的
`N Steps` 计数；Say/Done/Error settle 后仍回到原来的折叠逻辑，用户点开
阶梯（`user_touched`）后照旧渲染全部 step 行。

## 行为口径

- 新增谓词 `running_step_only(open, user_touched) = open && !user_touched`
  （`chat_step_live.rs`）：为真表示组行是**自动**打开的，只展示最后一个
  step 行；用户点击过（`user_touched`）则视为显式展开，全量渲染。
- 三处行号镜像同步 skip：`chat_step_render::flatten_step_group`（绘制）、
  `chat_headers::collect_headers`（点击命中）、
  `chat_steps::visible_targets`（`toggle_tool_call_at` 的索引解析），
  隐藏的历史 step 不产生点击目标，行号不漂移。
- 组行的 `N Steps` 总数、运行 spinner 与 Say 合并头保持不变；settle 后
  `Say(n steps): <preview>` 折叠对与改动前完全一致。
- 与 SPA 对齐：`steps/useLadderDisclosure.js` 的运行位本来只自动打开
  `steps` / `step:<最后一步>` / `calls:<最后一步>`，TUI 本次收敛到同一
  契约；SPA 无需改动。

## 变更清单

- `crates/tui/src/chat_step_live.rs` — 新增 `running_step_only` 谓词。
- `crates/tui/src/chat_step_render.rs` — `flatten_step_group` 接收
  `user_touched`，开启时从 `steps.len() - 1` 开始渲染。
- `crates/tui/src/chat_headers.rs` — `collect_headers` 用同一谓词跳过
  隐藏 step 的行号与 `call_idx`。
- `crates/tui/src/chat_steps.rs` — `visible_targets` 新增 `user_touched`
  参数并按同一规则 skip；`chat.rs::toggle_tool_call_at` 同步传参。
- `crates/tui/src/chat_flatten.rs` — 解构 `user_touched` 并下传渲染器。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行位只渲染最后一个 step（旧行隐藏、计数保留） | `live_running_position_renders_only_the_last_step` | `crates/tui/src/chat_tests/step_group/live_disclosure.rs` |
| Say settle 回落到折叠的 `Say(n steps)` 对 | `live_settle_folds_the_running_step_back_into_the_say_pair` | `crates/tui/src/chat_tests/step_group/live_disclosure.rs` |
| `visible_targets` 自动态仅暴露最后一步目标 | `visible_targets_mirror_the_three_level_ladder` | `crates/tui/src/chat_steps.rs` |
| 行号镜像在自动态下对齐（flatten ↔ headers） | `auto_running_position_keeps_alignment` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |
| 用户展开的阶梯仍覆盖全部深度 | `step_group_ladder_depth_alignment`、`step_group_with_expanded_call_keeps_alignment` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |

定向验证：`cargo test -p opencoder-tui --lib`（1812 passed；唯一失败
`ts_mirror::tests::maybe_wrap_gates_on_existing_registry` 由工作区内未提交
的 `ts_registry` 原型改为 RocksDB 目录、不再创建 `ts.db` 文件导致，与本次
改动无关）。
