# TUI 运行中实时区头行文案改为 `> streaming` / `▸ streaming`

一句话：TURN LIVE REGION 的可见头行标签由 `live` 改为 `streaming`
（前缀不变：展开 `> streaming`、收起 `▸ streaming`）；内部
`live` / `live_collapsed` / `live_headers` 等标识与区域概念不变，
仅调整用户可见文本及相关注释、断言与文档。

## 行为口径

- 展开态头行：`> streaming`，其下仍为当前 in-flight step 的明细行。
- 收起态头行：`▸ streaming`，仍是该 turn 运行期间的唯一区域行。
- 点击头行 / Ctrl+L 的收起与重新展开行为不变；`LIVE_ROW_OPEN_PREFIX`
  仍为 `> `，`GROUP_ROW_CLOSED_PREFIX` 仍为 `▸ `，仅标签字符串改为
  `streaming`。
- 历史 changelog（`2026-11-09`）保留当时的旧标签记录，不回写历史。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 实时区头行位于阶梯下方、Subagent 之上 | `live_region_sits_below_the_ladder_and_above_subagent_blocks` | `crates/tui/src/render_tests/live.rs` |
| 首个 in-flight step 期间头行是唯一表面 | `first_in_flight_step_is_the_turns_only_surface` | `crates/tui/src/render_tests/live.rs` |
| 统一滚动条与头行可见性 | `unified_scrollbar_covers_content_and_live` | `crates/tui/src/render_tests/live.rs` |
| 超长实时区滚动尾/头行可达 | `huge_live_rides_the_scroll_window_tail_kept_and_start_reachable` | `crates/tui/src/render_tests/live.rs` |
| 转录溢出时头行保持可见 | `live_strip_stays_visible_when_transcript_overflows` | `crates/tui/src/render_tests/live.rs` |
| 展开/收起行形（`> streaming` / `▸ streaming`） | `live_region_toggles_between_expanded_and_collapsed_rows` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| 每个新 turn 重新展开实时区 | `every_newly_admitted_turn_re_expands_the_live_region` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| Ctrl+L 收起后仍保留头行 | `collapse_all_keeps_the_live_region_visible` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| LiveHeader 行索引与渲染头行对齐 | `live_header_line_index_points_at_the_rendered_live_row` | `crates/tui/src/chat_tests/step_group/live_preview.rs` |
| 运行中阶梯行数镜像与头行位置 | `step_group_running_call_keeps_alignment` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |
| 实时区内联于阶梯下方 | `live_region_rides_inline_below_the_ladder` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |
| 点击索引落在实时区头行 | `step_row_line_index_lands_on_the_step_row` | `crates/tui/src/chat_tests/tool_collapse/turn_routing.rs` |
| 运行中 hint / 终态实时区头行 | `running_hint_persists_until_say_begins`、`terminal_event_clears_progress_when_no_say_arrives` | `crates/tui/src/chat_tests/tool_collapse.rs` |
| 首个 step 的实时区唯一表面 | `expanded_step_keeps_streaming_new_thinking_deltas` | `crates/tui/src/chat_tests/step_group.rs` |
| copy 模式排除实时区头行 | `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text` | `crates/tui/src/copy_mode/clean_say_tests.rs` |
