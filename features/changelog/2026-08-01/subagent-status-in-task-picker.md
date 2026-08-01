Commit: (working-tree, pre-initial-commit)

# feat(tui): /task 选择器显示子代理状态徽标 + resume 时重放横幅

## 背景

- 任务选择器只显示标题/预览与 `⇲ sub` 标记，无法感知某个会话还有子代理在跑
  （Running）或残留 Cancelled 待重放——用户只能盲选，或靠进会话后观察。
- resume 一个含 Running 子代理的会话时，`resume_and_replay` 会先重放子代理
  （回填一条 ToolResult）再继续，期间界面无任何提示，看起来像"卡住"。
- 本变更只做"可见性"：不改 `sessions` 表结构、不改 replay 语义（Running 子代理
  仍 eager 重放；Cancelled 仍留在下次用户 turn 才重放）。

## 变更

### store 层（`crates/store/src/types.rs`、`libsql_store/sessions.rs`）

- `SessionListItem` 新增派生计数字段 `pub subagent_running: usize`、
  `pub subagent_cancelled: usize`（均 `#[serde(default)]`，web/CLI 序列化兼容；
  `Store` trait 签名不变，字面构造点仅 `list()` 与测试 helper）。
- `list()` SELECT 追加两个相关子查询（复用既有索引 `idx_subagent_parent`，无
  GROUP BY，不扰动排序）：
  `(SELECT COUNT(*) FROM subagent_tasks st WHERE st.parent_session_id = s.id AND st.status = 'running')`
  与 `'cancelled'` 变体；行映射 `r.get::<i64>(8/9)?.max(0) as usize`。
- `status` 存 lowercase 字符串，与 `SubagentStatus::as_str()`（"running"/"cancelled"）一致。

### TUI 层（`crates/tui/src/task.rs`）

- 行渲染：在 `(current)` / `⇲ sub` 后缀之前插入徽标
  `● N running`（`theme::warn_color`）与 `⊗ N replay pending`（`theme::muted`），
  仅在对应计数 > 0 时出现。
- 宽度预算：`POPUP_W` 常量（60），`fixed` = agent chip + 分隔 + 徽标 + 后缀标签；
  `free = POPUP_W - fixed`；`title_budget = (free*2/3).clamp(10,28)`、
  `preview_budget = max(free - title_budget, 8)`；`short_preview(s, max_w)` 改按
  预算截断（`composer::str_width` / `truncate_to_width`）。

### TUI 层（`crates/tui/src/app_task.rs`、`app.rs`）

- `switch_session` 新增 `terminal: &mut crate::render::Term` 参数（app.rs 调用点
  传入 `run_app` 的 `terminal`）。
- Resume 分支在 `resume_and_replay` 之前调用新函数
  `draw_resume_replay_banner(terminal, store, id)`：经 `store.list_subagent_tasks`
  统计 Running 数，为 0 时 no-op，否则绘制居中 `Clear` + 加粗 warn
  `"Resuming session — replaying N subagent(s)…"`。
- 绘制体拆为纯函数 `resume_banner_message(n) -> Option<String>`（0 返回 None）
  与后端无关的 `render_resume_replay_banner(frame, msg)`；入口泛化为
  `<B: Backend>`，使 store 桩 + `TestBackend` 可直接单测（生产侧仍传 `Term`）。

### 语义确认（`crates/session/tests/resume_cancelled_pending.rs`）

- 新增测试固化现状：Cancelled 子任务在 resume 时保持 Cancelled（不回填
  tool_result、`task-cancelled` tool_use 保持 dangling、0 次 LLM 调用），留待
  下一用户 turn 由 `replay_cancelled_tasks` 重放——与新增 UI 徽标/横幅一致。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| list() 聚合 running/cancelled 计数且 completed 不计入 | `list_aggregates_running_cancelled_completed_mix` | `crates/store/tests/subagent_status_counts.rs`（integration） |
| 子会话自身计数为 0（排除逻辑不受影响） | `list_with_subagents_reports_zero_counts_for_children` | 同上 |
| 徽标渲染 running + replay pending | `status_badges_render_running_and_replay_pending` | `crates/tui/src/task.rs`（unit，TestBackend 渲染断言） |
| 长标题/后缀标签共存时徽标预算不溢出 | `status_badges_survive_long_titles_and_suffix_tags` | 同上 |
| preview 按自定义宽度预算截断 | `short_preview_respects_custom_budget` | 同上 |
| Subagent 块 Completed→done/ok=task.ok | `subagent_block_completed_maps_to_done` | `crates/tui/src/session_ui/subagent_block_tests.rs`（unit） |
| Subagent 块 Cancelled→"(cancelled)" 标记 | `subagent_block_cancelled_maps_to_cancelled_marker` | 同上 |
| Subagent 块 Failed→done/!ok | `subagent_block_failed_maps_to_failed` | 同上 |
| Subagent 块 Running→"(interrupted)" 终态 | `subagent_block_running_maps_to_interrupted` | 同上 |
| resume 后 Cancelled 保持待重放（不 eager 回填） | `resume_and_replay_leaves_cancelled_tasks_pending_replay` | `crates/session/tests/resume_cancelled_pending.rs`（integration，共用 `tests/common/mod.rs` fixture） |
| 横幅文案 n=0 → None（无横幅） | `resume_banner_message_none_for_zero` | `crates/tui/src/app_task.rs`（unit，纯函数） |
| 横幅文案按 Running 数计数（n=1/3） | `resume_banner_message_counts_subagents` | 同上 |
| Running 子代理存在时绘制横幅 | `resume_banner_drawn_when_running_subagents` | 同上（store 桩 + TestBackend 渲染断言） |
| 仅 Cancelled 子代理时 no-op（不触发横幅） | `resume_banner_noop_without_running_subagents` | 同上 |
| store 查询失败降级为 no-op | `resume_banner_noop_when_store_query_fails` | 同上 |
| 窄区域下横幅宽度/高度钳制不溢出 | `banner_renders_within_narrow_area` | 同上 |

> unit 层零 I/O / DB / 网络依赖（`EmptyChildStore` 桩）；store 与 session 测试为
> 真 libsql 内存库 integration 测试。resume 测试拆分到 `resume_cancelled_pending.rs`，
> fixture 抽到 `tests/common/mod.rs`，`resume_replay.rs` 保持在 800 行内。

## 全量回归

| 检查 | 结果 |
|------|------|
| `cargo check -p opencoder-store` | PASS |
| `cargo check -p opencoder-tui`（非 test） | PASS |
| `cargo test -p opencoder-store --test subagent_status_counts` | PASS — 2 passed / 0 failed |
| `cargo test -p opencoder-tui --lib task::`（含 app_task banner 6 项） | PASS — 16 passed / 0 failed |
| `cargo test -p opencoder-tui --lib session_ui::`（含 `subagent_block_tests` 4 项） | PASS — 16 passed / 0 failed |
| `cargo test -p opencoder-tui --lib app_task::`（banner 新增 6 项） | PASS — 6 passed / 0 failed |
| `cargo test -p opencoder-tui --lib` 全量 | PASS — 726 passed / 0 failed |
| `cargo test -p opencoder-session --test resume_replay` + `--test resume_cancelled_pending` | PASS — 8 + 1 passed / 0 failed |
| `cargo test --workspace` 全量 | PASS — 96 个测试目标、1525 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 0 warnings |
