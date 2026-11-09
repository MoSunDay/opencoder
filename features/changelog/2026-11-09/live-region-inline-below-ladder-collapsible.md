# live 区内联到运行中阶梯下方；`> live` 头行默认展开、可收起

一句话：两条收口。(1) **内联几何**：turn live region 不再由
`render.rs` 拼在转录滚动窗尾部（`render_live.rs` 模块删除），改由
`chat_flatten.rs::flatten_with` 在运行中 StepGroup（`progress_active`，
Say 未开始）的阶梯行之后**内联**输出，天然位于后续 Subagent 块之上，
与转录共用同一滚动窗/滚动条（ViewportCache 统一计行）。(2) **settled
门控 + 可收起头行**：阶梯只渲染已落定 step（`settled_steps`）——首个
in-flight step 流式期间阶梯零行，live 区是该 turn 唯一表面；头行由
`⬵ live` 改为 `> live`（展开）/ `▸ live`（收起），默认展开、点击头行
或 Ctrl+L 收起、每个新 turn 重新展开。

## 行为口径

- 内联几何：`flatten_with` 在运行中 StepGroup 的阶梯行之后内联输出
  live 区，位于后续 Subagent 块之上；与转录共用同一滚动窗/滚动条
  （ViewportCache 统一计行）。`render_live.rs` 删除，render.rs 不再
  拼窗。
- settled 门控：`chat_steps.rs::settled_steps` 阶梯只渲染已落定 step，
  运行中的最后一个 in-flight step 只出现在 live 区、不进阶梯。首个
  step 流式期间阶梯**零行**（连 `N Steps` 行都没有，live 区是该 turn
  唯一表面）；Say 开始或 turn 结束（settle）后 live 区消失，完整 step
  落入阶梯历史。渲染（`chat_step_render.rs`）、行数镜像
  （`chat_headers.rs`）、点击目标（`chat.rs`
  toggle_tool_call_at/visible_targets）三处同门控。
- 头行与收起：头行 `> live`（`LIVE_ROW_OPEN_PREFIX`）展开 /
  `▸ live`（复用 `GROUP_ROW_CLOSED_PREFIX`）收起，保持 dim warn 样式；
  `chat_live_turn.rs::render_lines(steps, anim_tick, collapsed)` 收起时
  仅头行。`ChatView::live_collapsed` 默认 false（展开）；
  `toggle_live_region()` 翻转；`collapse_all_collapsible`（Ctrl+L）置
  true；`begin_turn` / `reanchor_turn_after_user_echo`（每次新 turn 的
  arm 点）重置 false——每个新运行 turn 默认展开。
- 点击：`chat_headers.rs::collect_headers` 在 live 区首行登记
  `LiveHeader { block_idx, header_line_idx }`（与 flatten 行严格对齐）；
  `render_hits.rs` 新增 `LiveBtn`/`MouseHits.live_btns`/
  `record_live_hits`；`app_mouse.rs` 左键命中调 `toggle_live_region()`
  （subagent-aware：collapse_view 路由）。
- copy 模式隔离：`flatten_with` 拆出 `flatten_opts(.., include_live)`，
  新增 `flatten_for_copy` 不含 live 区（复制文本保持纯转录，不带
  `> live` 头行与流式 spinner）；`render_viewport.rs` 加
  `build_for_copy`/`includes_live`/`refresh_copy_cache`，
  `copy_mode/mod.rs::render_clean` 用之，离开 copy 模式时 render.rs
  重建装饰版缓存。
- 事件状态机零改动；SPA 零改动（其 live 几何本就内联于阶梯下方）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| live 区位于阶梯下方、Subagent 块之上 | `live_region_sits_below_the_ladder_and_above_subagent_blocks` | `crates/tui/src/render_tests/live.rs` |
| 首个 in-flight step 期间阶梯零行（live 区是该 turn 唯一表面） | `first_in_flight_step_is_the_turns_only_surface` | `crates/tui/src/render_tests/live.rs` |
| 流式 Say 独立渲染至 seal | `streaming_say_renders_standalone_in_the_transcript_until_sealed` | `crates/tui/src/render_tests/live.rs` |
| 统一滚动条覆盖转录 + live | `unified_scrollbar_covers_content_and_live` | `crates/tui/src/render_tests/live.rs` |
| 展开/收起行形（`> live` / `▸ live`） | `live_region_toggles_between_expanded_and_collapsed_rows` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| 每个新 turn 重新展开 live 区 | `every_newly_admitted_turn_re_expands_the_live_region` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| Ctrl+L 收起 live 区后仍可见头行 | `collapse_all_keeps_the_live_region_visible` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| LiveHeader 行索引与渲染行严格对齐 | `live_header_line_index_points_at_the_rendered_live_row` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| copy 模式经 flatten_for_copy 排除 live 区 | `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text` | `crates/tui/src/copy_mode/clean_say_tests.rs` |
| 阶梯 settled 门控行数镜像（live 内联其下、in-flight 不进阶梯） | `live_region_rides_inline_below_the_ladder` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |
| 点击目标 settled 门控（首个 step 流式期 live 头行是唯一目标） | `step_row_line_index_lands_on_the_step_row` | `crates/tui/src/chat_tests/tool_collapse/turn_routing.rs` |

- 定向回归：`cargo test -p opencoder-tui --lib -- render_tests::live chat::tests::step_group chat::tests::tool_collapse chat::tests::say_pair chat::tests::subagent_dispatch_say copy_mode mouse_tests mouse_helpers chat::tests::line_accounting` → 133 passed / 0 failed
- Clippy：`cargo clippy -p opencoder-tui --all-targets` → 零警告
- 测试范围：仅本次改动及直接影响链路（聊天渲染/点击/copy 模式）；未运行全量测试。
