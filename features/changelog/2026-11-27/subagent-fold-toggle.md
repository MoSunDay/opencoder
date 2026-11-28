# Subagent 分组披露：`> subagents` / `▸ subagents` 组头行一键折叠（TUI）；Web 保持块级 `❯`/`▸` 同步开合

一句话：TUI 每段极大连续的 Subagent 块前置**一行**与 turn 实时区同款样式的分组头——`> subagents`
展开态（其下每行 `⤷ subagent …` 保持完整明细）、`▸ subagents` 收起态（只留组头，subagent 行不渲染）；
点击组头一次折叠 / 展开当前视图的**全部** subagent 分组，Ctrl+L 同样收起；Web SPA 维持既有契约：
subagent 块头部显示阶梯同款 `❯`/`▸`，点击任一头部同时开 / 合全部 subagent 块，Ctrl/Cmd+L 与
`⤒ 收起` 一并收起（Web 没有组头行）。

## 背景与决策

- 用户反馈：subagent 行没有阶梯 / 实时区那样的 `> ` → `▸ ` 折叠手段，长会话里多个 subagent
  的提示词 + 摘要行占屏；希望「大 > 点击成小 > 收起所有 subagent，再点展开，Ctrl+L 也收起」。
- 第一版把标记加在每个 subagent 行首（`> ⤷ subagent …` / `▸ ⤷ subagent …`，行首两列为折叠目标，
  收起态裁成紧凑行）。用户纠正：应当像 live 区的 `> streaming` 那样只有**一行**组头
  `> subagents`，点击组头折叠 / 展开整个分组；行本身不带标记，展开态始终是完整明细。
- 决策：折叠是**视图级**状态而非逐块状态（「收起所有」）；分组头按「极大连续 Subagent 块段」各
  一行（非 Subagent 块分段），样式与 live 区头行完全一致（`warn_color` + `DIM`）。收起态 subagent
  行**不渲染**（不是裁剪），因此行数随收起减少、`subagent_headers()` 收起态为空；组头两种状态都
  存在，命中目标不丢。状态跨 turn 持久（与 Thinking 折叠同类，`begin_turn` 不重置），Ctrl+L
  只收起不展开。组头是转录结构（区别于瞬态 live 区），copy 模式保留。
- Web：`TranscriptView` 持有唯一的 `{ open, toggle }`（默认收起，保持现有「子事件默认隐藏」
  的形态），经 Context 下发；不放进 epoch 重挂的气泡里，所以 `collapseAll` 显式把它置为收起。

## 行为与实现

### TUI

- `crates/tui/src/chat.rs`：`SUBAGENT_GROUP_LABEL = "subagents"`（取代第一版的
  `SUBAGENT_MARKER_COLS`）；`toggle_subagents()` 切换 `subagents_collapsed`；
  `collapse_all_collapsible` 同时置 `subagents_collapsed = true`（Ctrl+L 路径
  `app_helpers.rs::pre_key_intercept` 无需改动：它先对聚焦子视图、再对父视图调用该函数）。
- `crates/tui/src/chat_types.rs`：`ChatView::subagents_collapsed: bool`（紧邻 `live_collapsed`，
  文档改为分组语义）；`Subagent` 变体文档改述「组头 + 完整行」；新增
  `SubagentGroupHeader { header_line_idx }`。
- `crates/tui/src/chat_flatten.rs`：`flatten_opts` 在进入每段连续 Subagent 段前 push 一行
  `subagent_group_header_line(collapsed)`（`{prefix}{SUBAGENT_GROUP_LABEL}`，
  `> ` / `▸ ` 前缀，`Style::default().fg(theme::warn_color()).add_modifier(Modifier::DIM)`，
  与 `chat_live_turn.rs` 同款）；`in_subagent_run` 在 `match` 之前更新，`continue` 分支不会
  失步；Subagent 行恢复到折叠前形态（`⤷ subagent [kind] prompt …` + 时长 + ` [→ view]` +
  摘要，无前缀），`subagents_collapsed` 时整行跳过。`flatten_for_copy` 同样输出组头。
- `crates/tui/src/chat_headers.rs`：`collect_headers` 镜像计行（每段组头 +1；行仅展开态 +1，
  `prev_was_sub` 在 `match` 前更新），返回 6 元组追加 `Vec<SubagentGroupHeader>`（既有下标不变，
  `live_headers()` 仍为 `.4`）；新增 `subagent_group_headers()`（`.5`）。
- `crates/tui/src/render_hits.rs`：`MouseHits::subagent_group_btns: Vec<SubagentGroupBtn>`、
  `record_subagent_group_hits`（镜像 `record_live_hits`：每个可见组头一条、全文本宽、1 行高）。
- `crates/tui/src/render.rs`：两处 clear 站点清空 `subagent_group_btns`，`render_body` 透传并记录。
- `crates/tui/src/app_mouse.rs`：`subagent_group_btns` 命中 →
  `collapse_view(chat, focus).toggle_subagents()`，消费点击且不碰 scroll / follow / focus；
  `subagent_btns` 命中恢复为「整行进入视角」，删除行首标记分支。

### Web（本轮未改动）

- `crates/web/spa/src/subagentFold.js`：`SubagentFoldContext` + `useSubagentFold()`
  （Provider 内返回转录级 `{ open, toggle }`，否则回退块本地开关）。
- `crates/web/spa/src/transcript.jsx`：`subagentsOpen` state + `collapseAll`
  （epoch bump + 收起 subagent），keydown 与 `⤒ 收起` 共用；`Bubble.List` 包在 Provider 内。
- `crates/web/spa/src/stepsBlock.jsx`：导出 `disclosureIcon`；`subagentBlock.jsx` 改用
  `useSubagentFold()`，Collapse 受控于转录级开关并显示 `❯`/`▸`；`[→ view]` 仍阻止冒泡。
- Web 的披露仍是**逐块头部共享**的 `❯`/`▸`（点击任一头部同步全部块），没有 `> subagents`
  组头行，也不隐藏头部本身。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 默认展开：`> subagents` 组头 + 两行完整明细、copy 模式保留组头 | `subagent_run_renders_the_open_group_header_then_full_rows_by_default` | `crates/tui/src/chat_tests/subagent_fold.rs` |
| 组头样式与 live 区头行一致（warn + DIM，两种状态） | `subagent_group_header_matches_the_live_region_header_style` | `crates/tui/src/chat_tests/subagent_fold.rs` |
| 一次切换把整段收成仅 `▸ subagents` 一行并可切回 | `toggle_subagents_collapses_the_run_to_its_header_only` | `crates/tui/src/chat_tests/subagent_fold.rs` |
| Ctrl+L 收起 subagent 且跨 turn 持久（`begin_turn` 不重置） | `collapse_all_collapsible_collapses_subagents_and_the_state_persists_across_turns` | `crates/tui/src/chat_tests/subagent_fold.rs` |
| `subagent_group_headers()` 两种状态各一条且指向组头行；`subagent_headers()` 展开态 2 / 收起态 0 | `subagent_group_headers_and_row_headers_track_the_rendered_lines` | `crates/tui/src/chat_tests/subagent_fold.rs` |
| 每段连续 Subagent 块各有组头，非 Subagent 块分段；收起后后续组头行号随之上移 | `each_contiguous_run_opens_with_its_own_group_header` | `crates/tui/src/chat_tests/subagent_fold.rs` |
| 计行镜像：组头每段 +1、行仅展开态 +1，收起后下方命中行号对齐 | `subagent_group_header_and_rows_align_in_both_states` | `crates/tui/src/chat_tests/line_accounting/subagent_group.rs` |
| 点击组头切换全部、不进入视角、不动 scroll / follow；再点展开 | `clicking_the_group_header_toggles_the_subagent_rows_without_entering_the_view` | `crates/tui/src/app_helpers_tests/mouse_tests/subagent_fold.rs` |
| 点击可见行进入视角且不切换折叠（行首列不再是折叠目标） | `clicking_a_visible_row_enters_the_view_without_toggling_the_fold` | `crates/tui/src/app_helpers_tests/mouse_tests/subagent_fold.rs` |
| 收起态命中表只有组头：隐藏行点击无效，组头仍可展开 | `collapsed_clicks_only_the_group_header_remains_actionable` | `crates/tui/src/app_helpers_tests/mouse_tests/subagent_fold.rs` |
| 命中表：展开态组头 + 行，收起态仅组头 | `hit_map_carries_the_group_header_and_rows_only_while_expanded` | `crates/tui/src/app_helpers_tests/mouse_tests/subagent_fold.rs` |
| 聚焦子视图内的组头点击只切子视图 | `group_header_click_inside_a_focused_subagent_toggles_the_child_view` | `crates/tui/src/app_helpers_tests/mouse_tests/subagent_fold.rs` |
| Ctrl+L 在父视图与聚焦子视图都收起 subagent（`▸ subagents`） | `ctrl_l_collapses_subagent_rows_in_parent_and_focused_child` | `crates/tui/src/app_helpers_tests/ctrl_l_tests.rs` |
| `record_subagent_group_hits`：两种状态都有组头目标、行目标仅展开态；滚动后目标随内容上移 | `group_header_target_in_both_states_rows_only_while_expanded` / `group_header_scrolled_above_is_not_hittable` | `crates/tui/src/render_tests/subagent_group.rs` |
| 整帧 `render` 记录组头 / 行目标，收起后重渲染不残留行目标 | `full_frame_records_group_header_and_row_targets_per_state` | `crates/tui/src/render_tests/subagent_group.rs` |
| `subagent_group_headers()` 指向 `> subagents` 行且紧邻首个 subagent 行 | `header_line_indices_aligned_with_flatten_with_preamble_visible` | `crates/tui/src/render_tests/chips.rs` |
| 既有精确 flatten 期望同步（组头行 + 行去前缀 + 行号 +1） | `streaming_say_after_a_subagent_retires_the_ladder_behind_the_subagent_row` 等 4 例 / `subagent_dispatch_header_renders_markdown_and_body_does_not_repeat` 等 2 例 | `crates/tui/src/chat_tests/subagent_steps_continue.rs` / `subagent_dispatch_say.rs` |
| Web（既有覆盖，本轮未改动未重跑）：默认 `▸` 且子事件隐藏 | `renders the ▸ marker on every subagent block and hides child lines by default` | `crates/web/spa/src/subagentFold.dom.test.jsx` |
| Web：点击一个头部同时开 / 合全部 | `clicking one subagent header toggles every subagent block open, then closed` | `crates/web/spa/src/subagentFold.dom.test.jsx` |
| Web：Ctrl+L 收起全部 | `Ctrl+L collapses every subagent block` | `crates/web/spa/src/subagentFold.dom.test.jsx` |
| Web：`⤒ 收起` 收起全部 | `⤒ 收起 collapses every subagent block` | `crates/web/spa/src/subagentFold.dom.test.jsx` |
| Web：`[→ view]` 不触发折叠 | `the [→ view] drill-in link does not toggle the disclosure` | `crates/web/spa/src/subagentFold.dom.test.jsx` |
| Web：Provider 外回退块本地开关 | `falls back to a block-local toggle outside TranscriptView` | `crates/web/spa/src/subagentFold.dom.test.jsx` |

- 定向回归（TUI）：`cargo test -p opencoder-tui --lib subagent_fold` → 11 passed；
  `--lib subagent` → 95 passed（含 `render::tests::subagent_group` 3 例、`line_accounting::subagent_group`
  1 例与既有 subagent 用例）；`--lib subagent_group` → 6 passed；`--lib ctrl_l` → 13 passed；
  `--lib clicking_` → 4 passed；`--lib collapse` → 50 passed；`--lib line_accounting` → 15 passed；
  另 `copy_mode` 41 / `mouse` 39 / `live_region` 9 / `thinking` 42 / `hierarchy` 4 / `steer_actions` 2 /
  `step` 72 / `dispatch` 45 / `clear` 94 / `render::tests::body` 8 / `render::tests::live` 7 passed，
  均 0 failed。
- 定向回归（Web）：本轮 Web 代码未改动、未重跑；上一轮记录
  `npx vitest run src/subagentFold.dom.test.jsx src/subagentBlock.dom.test.jsx src/stepsBlock.dom.test.jsx
  src/chat.dom.test.jsx src/todo/review/conversation/workspace.dom.test.jsx
  src/todo/review/session.dom.test.jsx` → 6 files，69 passed / 0 failed。
- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告；
  `cargo fmt -p opencoder-tui -- --check` 无差异。
- 构建：Web dist 未改动（沿用上一轮 `scripts/build-spa.sh` 产物）。
- 测试范围：本次修改及其直接影响链路；未运行全量 Rust / SPA 测试与浏览器验收。

## Impact Surface

- TUI：每段连续 Subagent 块前新增一行 `> subagents` / `▸ subagents` 组头（dim warn）；
  subagent 行本身不带标记，展开态恒为完整明细（时长、` [→ view]`、摘要）；收起态行不渲染
  （行数减少、`subagent_headers()` 为空，组头目标保留）。点击组头切换折叠，点击行进入视角；
  copy 文本含组头行。
- Web：subagent 头部图标为 `❯`/`▸`；开合为全部同步；默认仍收起；无组头行。
- 未改 API / CLI / 依赖 / Ontology 目录。
