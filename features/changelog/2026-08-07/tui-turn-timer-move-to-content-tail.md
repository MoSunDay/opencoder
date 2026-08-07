Commit: (working-tree, pre-initial-commit)

# TUI turn 计时器：从状态栏移至内容区最后一行尾部 + subagent 视图计时

## 背景

用户反馈 turn 计时器位置不对：计时器应**永远显示在信息展示区域（body）最后一行内容的尾部**，
而非状态栏（status bar）。此外，当聚焦到 subagent 视图时，subagent 内部的每一轮交互也应计时并
在内容尾部展示。

此前计时器渲染在 `render_status` 的状态行末尾（参见 `tui-status-bar-per-turn-timer.md`），
与用户要求的"内容区最后一行尾部"不符；subagent 视图则完全没有独立计时。

## 变更

### 1. 计时器从状态栏移至 body 内容尾部

- **`crates/tui/src/render.rs`** `render_body`：
  - 新增 `turn_ms: u64` 参数。
  - 在 viewport 切片可见行后、渲染 Paragraph 前，当 `turn_ms > 0` 且最后一行内容在可视区域内
    （`end == n`）时，找到最后一条含非空白文本的行，在其 spans 尾部追加
    `Span::styled(format_run_duration(turn_ms), warn_color)`。
  - 计时器以 10 FPS 实时更新（post-slice 追加，不依赖 3 FPS body cache 重建）。
- **`crates/tui/src/render.rs`** `render_status`：移除 `run_ms` 参数及计时器渲染块，
  状态栏不再显示 turn 计时。
- **`crates/tui/src/render.rs`** `render()`：`render_body` 调用传入 `run_ms`，
  `render_status` 调用移除 `run_ms`。

### 2. subagent 视图独立计时

- **`crates/tui/src/app.rs`**：
  - 提取 `now = now_ms()`（消除重复调用）。
  - `render_frame` 的 `now_ms` 参数改用提取的 `now`，`run_ms` 参数改用 `display_turn_ms`。
- **`crates/tui/src/app_display.rs`**：
  - 新增纯函数 `display_turn_ms(chat, subagent_focus, run_elapsed_ms, now) -> u64`：
    聚焦 subagent 且其 `done == false` 时使用 `now - started_at_ms`（实时）；
    否则回退到 `run_elapsed_ms`。从 `app.rs` 提取以保持 app.rs ≤ 800 行且可独立单测。
  - 新增 3 个单元测试覆盖该函数的全部分支（见测试覆盖表）。

### 3. 测试更新

- **`crates/tui/src/render_tests/timer.rs`**：完全重写，改为测试 body 内容尾部计时
  （`body_shows_turn_timer_at_content_tail` / `body_hides_turn_timer_when_zero` /
  `body_turn_timer_after_content`）。
- **`crates/tui/src/render_tests/body.rs`**：7 处 `render_body` 调用补 `turn_ms = 0`。
- **`crates/tui/src/render_tests/status_ctx.rs`**、**`status_bar.rs`**：移除
  `render_status` 调用的 `run_ms` 尾参。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| body 尾部显示计时 | `body_shows_turn_timer_at_content_tail` | `crates/tui/src/render_tests/timer.rs` |
| turn_ms=0 不显示 | `body_hides_turn_timer_when_zero` | `crates/tui/src/render_tests/timer.rs` |
| 计时在内容文本之后 | `body_turn_timer_after_content` | `crates/tui/src/render_tests/timer.rs` |
| 运行中 subagent 实时计时 | `running_subagent_shows_live_elapsed` | `crates/tui/src/app_display.rs` |
| done subagent 计时归零 | `done_subagent_returns_zero` | `crates/tui/src/app_display.rs` |
| 无 subagent 聚焦回退 run_elapsed | `no_subagent_focus_falls_back_to_run_elapsed` | `crates/tui/src/app_display.rs` |
| tick_clock 边界逻辑（未改） | `tick_clock_resets_elapsed_on_turn_*` | `crates/tui/src/app_loop_bugfix_tests.rs` |

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 编译干净
- 行数：`render.rs` 795 ≤ 800；`app.rs` 797 ≤ 800；`app_display.rs` 102 ≤ 800

## Impact Surface

- 用户可感知：turn 运行时计时器出现在 body 最后一条内容行的尾部（warn 色实时跳动）；
  聚焦 subagent 时该视图尾部也有独立计时。
- 状态栏不再显示 turn 计时（ctx meter / spinner / status 不受影响）。
- 不影响：CLI / Web / session / store / runner 边界；计时仍为纯展示层，不持久化。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [既有 changelog：per-turn 计时器引入](tui-status-bar-per-turn-timer.md)
- [既有 changelog：Tool/Subagent duration timer](tui-tool-subagent-duration-timer.md)
