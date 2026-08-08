Commit: (working-tree, pre-initial-commit)

# TUI 计时器拆分：尾部显示「一轮 function call」耗时，状态栏显示任务总时长

## 背景

用户对 TUI 计时器提出三点修正：

1. **状态栏底部**应显示任务执行总时间，位置从 ctx 后**移到 running 动画右边**，
   颜色从 muted 改为 **warn 橙**。
2. **消息尾部**改为显示「一轮 function call 的耗时」（一轮可能多次连续 call，
   实时累计），格式 `[call 42s]`；一行放不下时**必须换行到独立行**，不能丢。
3. **取消 Tool header 上的单次 call 内联计时**；Subagent header 计时保留
   （subagent 是长任务，需实时可见）。

此前尾部计时由 `run_elapsed_ms` 累加器驱动（每轮 turn 一次），且 Tool header 上
每个 call 都有内联计时，与「一轮 call」的语义不符。

## 变更

### 1. 尾部计时改为 call-round 语义（纯数据推导，删除累加器）

- **`crates/tui/src/app_display.rs`**：
  - `display_turn_ms`（4 参，含 `run_elapsed_ms`）→ `display_tail_ms(chat, subagent_focus, now)`
    （3 参）：subagent focus 分支保留 live elapsed；非 focus 分支改由
    `call_round_ms` 推导。
  - 新增纯函数 `call_round_ms`：rposition 找最后 running Tool（`elapsed_ms == None`）→
    向前扫连续 Tool 段 → 取段首 `started_at_ms` 到 now 的毫秒数；无 running tool 返回 0；
    非 Tool block 截断 round。
- **`crates/tui/src/app_loop.rs`**：`tick_clock` 删除 `run_elapsed_ms` 参数（5→4 参）；
  `false→true` 仅 snap baseline（排除 idle 间隙），不再重置任何计时；doc 重写为 task-only。
- **`crates/tui/src/app.rs`**：L92 单变量 `task_elapsed_ms`；tick_clock 调用去参；
  `display_tail_ms` 替换 `display_turn_ms` 调用；提交任务的既有两处 `task_elapsed_ms = 0`
  重置点保持不变。
- **`crates/tui/src/frame.rs` / `render.rs`**：`turn_ms` → `tail_ms` 签名同步
  （`render_frame` / `render` / `render_body`）。

### 2. 尾部 timer 渲染：`[call 42s]` + 放不下换行

- **`crates/tui/src/render.rs`** `render_body`（L417-441）：`[turn cost {}]` →
  `[call {}]`（warn 色）；当最后非空行 `width() + 2 + timer.width() > text_w` 时，
  `visible_lines.push(Line::from(timer))` 独立成行，绝不丢弃。

### 3. 状态栏任务总时长：位置 + 颜色修正

- **`crates/tui/src/render.rs`** `render_status`（L735-760）：删除 ctx 后 muted 的
  task 块；改为在 spinner/`· status` 之后追加 `  {format_run_duration(task_ms)}`，
  `theme::warn_color()`。

### 4. 取消 Tool 单次 call 内联计时

- **`crates/tui/src/chat.rs`**：删除 Tool collapsed（原 L509）与 expanded（原 L527）
  两处 `push_duration_span` 调用；L617 Subagent 调用保留；函数 doc 注明现仅 subagent
  使用。`ChatBlock::Tool` 的 `started_at_ms`/`elapsed_ms` 字段与 ToolStart/ToolEnd
  记录逻辑保留（`call_round_ms` 的数据源）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 尾部显示 `[call 42s]` | `body_shows_call_timer_at_content_tail` | `crates/tui/src/render_tests/timer.rs` |
| tail_ms=0 不显示 | `body_hides_call_timer_when_zero` | `crates/tui/src/render_tests/timer.rs` |
| 计时在内容文本之后 | `body_call_timer_after_content` | `crates/tui/src/render_tests/timer.rs` |
| 满行换行到独立行 | `body_call_timer_wraps_to_own_line_when_full` | `crates/tui/src/render_tests/timer.rs` |
| 任务时长在 spinner 右侧 + warn 色 | `status_bar_shows_task_time` | `crates/tui/src/render_tests/status_bar.rs` |
| task_ms=0 隐藏 | `status_bar_hides_task_time_when_zero` | `crates/tui/src/render_tests/status_bar.rs` |
| subagent live/done 计时（改签名） | `running_subagent_shows_live_elapsed` / `done_subagent_returns_zero` | `crates/tui/src/app_display.rs` |
| 无 running tool 返回 0 | `no_running_tool_returns_zero` | `crates/tui/src/app_display.rs` |
| 单 tool round 实时累计 | `running_tool_round_shows_elapsed` | `crates/tui/src/app_display.rs` |
| round 覆盖连续多 call（段首起算） | `round_spans_consecutive_tools_from_first_start` | `crates/tui/src/app_display.rs` |
| 非 Tool block 截断 round | `non_tool_block_breaks_the_round` | `crates/tui/src/app_display.rs` |
| tick_clock 不再重置 / 累加 / 跨 turn 保留 / false→true 排除 idle | `tick_clock_does_not_reset_task_on_turn_start` / `tick_clock_accumulates_task_while_running` / `tick_clock_preserves_task_across_turn_end_and_idle` / `tick_clock_false_to_true_excludes_idle_gap` | `crates/tui/src/app_loop_bugfix_tests.rs` |
| Tool header 无内联计时（删除 7 个测试） | （删除）`running_tool_*` / `done_tool_*` | `crates/tui/src/chat_tests/timer.rs` |
| Subagent header 计时保留 | `running_subagent_shows_live_timer` / `done_subagent_freezes_duration` / `done_subagent_hides_subsecond` | `crates/tui/src/chat_tests/timer.rs` |

- 全量回归：`cargo test --workspace` → 2023 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 编译干净
- 行数：`render.rs` 792 ≤ 800；`app.rs` 794 ≤ 800；`app_display.rs` 170 ≤ 800

## Impact Surface

- 用户可感知：body 尾部计时改为 `[call 42s]`（一轮连续 tool call 从第一发起实时累计，
  最后一发结束即消失）；Tool header 不再显示单次 call 计时；状态栏任务总时长移至
  running 动画右侧并改为 warn 橙。
- 不影响：CLI / Web / session / store / runner 边界；`ChatBlock::Tool` 字段与
  ToolStart/ToolEnd 事件记录逻辑不变；subagent focus 尾部计时行为不变。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [既有 changelog：per-turn 计时器引入](tui-status-bar-per-turn-timer.md)
- [既有 changelog：计时器移至内容尾部](tui-turn-timer-move-to-content-tail.md)
- [既有 changelog：Tool/Subagent duration timer](tui-tool-subagent-duration-timer.md)
