Commit: (working-tree, pre-initial-commit)

# TUI 渲染正确性 + session 健壮性批量修复

## 背景

一批独立发现的问题集中修复：(1) Ctrl+D 退出卡死在 alt-screen；(2) steer 输入回显进主执行区；(3) 软换行后光标位置错位；(4) plan 模式过度拦截只读重定向（`2>/dev/null`、`2>&1`）；(5) 单用户多工具轮次会话永不压缩、撑爆上下文窗口；(6) skill-only 空提示词提交不记录用户 turn；(7) 高 token 速率下渲染吃满 CPU；(8) 复制操作无可见反馈；(9) emoji/CJK 显示宽度计算不全；(10) `/task` 切回应过期子代理快照；(11) 输入线程 poll 失败时 stderr 破坏 alt-screen。

## 变更

### session：plan-mode 重定向分类精细化（`crates/session/src/bash_guard.rs`、`crates/core/src/agent.rs`）
- 原 `has_redirect` 一律拦截所有重定向操作符（`>`、`>>`、`&>`、`2>&1`）。现改为 `has_unsafe_redirect`：仅拦截**写文件**的重定向，放行只读重定向（`/dev/null` 丢弃输出、fd 合并 `2>&1`/`1>&2` 等 dup2）。
- 新增 `match_redirect_op` / `read_redirect_target` / `is_safe_redirect_target` 三个纯函数解析重定向操作符与目标，安全目标（`&N`、`/dev/null`）放行，其余（`> file`、`>> file`、`2> file`、`&> file`、`> /dev/null/sneaky`、`2>/dev/nullx`）拦截。复合命令中任一段的写文件重定向仍被捕获。
- `agent.rs::PLAN_SUFFIX` 文案 "redirects" → "file-writing redirects"。
- **file:line**: `bash_guard.rs:90-101`(分类入口)、`bash_guard.rs:115-191`(新解析函数)。

### session：压缩 turn 边界泛化（`crates/session/src/compaction.rs`）
- `split_index` 原仅把「真实 user 消息」作为可压缩 turn 边界——单用户多工具轮次会话（最常见的编码代理形态）只有 1 条 user 消息，永不触发压缩，大转录一直增长直到 provider 拒绝。
- 现把「紧跟 tool 消息之后的 assistant 消息」（工具轮次结束、模型新一轮响应）也作为 turn 边界；首条消息（index 0）也算边界。经典多用户会话的切分点不变（其 turn 边界集合与原来相同）。
- **file:line**: `compaction.rs:94-122`。

### session：skill-only 空提示词注入触发消息（`crates/session/src/runner.rs`）
- 空 prompt + `skill_prompt_cloned().is_some()` 时，注入一条 synthetic user 触发消息（"The active skill is now in effect. Begin executing it now."），使模型记录一个 user turn 并主动执行系统提示词中的 skill body，而非被动对待。web drain（无 skill）路径不变。
- **file:line**: `runner.rs:121-142`。

### session：cancel 解阻塞挂起流（`crates/session/tests/quit_while_running.rs`）
- 新增 `cancel_unblocks_a_hung_stream_promptly`：用永不发送事件的 `HungStream` 验证 cancel 使 `run` 在秒级经 biased `select!` cancel 臂返回，而非阻塞到请求超时。这是 TUI Ctrl+D 退出修复依赖的核心机制。

### session：子代理 × 交错思维交叉回归（`crates/session/tests/subagent_interleaved_thinking.rs`）
- 新增 4 个测试覆盖「子代理工具调用 turn 发出 ReasoningDelta」这一此前从未覆盖的交集路径：reasoning 持久化到子代理转录、第二次请求回传 reasoning_content、interleaved=false 跳过持久化、子代理继承 reasoning_effort。

### TUI：Ctrl+D 退出卡死（`crates/tui/src/app.rs`）
- `KeyAction::Quit` 臂：若 `running`，先 `cancel.cancel()` 中断当前 turn（runner 的 biased select! 臂立即返回 `Status("interrupted")`），push `[exiting…]` 标记，再 `send(Quit)+break`。
- `worker.await` 加 `tokio::time::timeout(5s, worker)` 超时兜底：即使某工具忽略 cancel，也能保证 `TerminalGuard::drop` 恢复终端。
- **file:line**: `app.rs:716-742`(Quit 臂)、`app.rs:841`(timeout)。

### TUI：steer 执行区不回显（`crates/tui/src/app.rs`、`crates/tui/src/session_ui.rs`）
- 删除 steer 臂的 `chat.push_marker("↳ steer: ...")`，使 steer 与 queue 一致——只在侧边暂存面板 + 状态栏展示。
- `replay_one` 对 `msg.synthetic == true` 的 user 消息跳过 `push_marker("user:")` 渲染（steer/queue 提升、plan→act handoff、压缩摘要等合成消息）。
- **file:line**: `app.rs:651-657`(steer 臂)、`session_ui.rs:90-99`。

### TUI：软换行光标错位 — 统一换行源（`crates/tui/src/composer.rs`、`crates/tui/src/render.rs`、`crates/tui/src/key_handler.rs`）
- 原两套不一致换行算法（光标用贪心逐字、渲染用 ratatui WordWrapper）导致含空格行换行点错位。
- 新增 `pub fn wrap_rows(input, inner_w, prompt_w) -> Vec<VisualRow>`：按词边界换行，首行窄 prompt_w、续行满 inner_w，显式 `\n` 必断行。作为渲染与光标的**唯一真相**。
- `cursor_row_col`、`display_rows`、`move_cursor_vertical` 全部改为基于 `wrap_rows` 推导；`move_cursor_vertical` 加 `inner_w`/`prompt_w` 参数修复 Up/Down 跨软换行。
- `render_composer` 用 `wrap_rows` 预切分构造显式 `Line` 列表，**关闭 ratatui `.wrap()`**，保证渲染与光标完全一致。

### TUI：帧率限制 — 解耦 CPU 与 token 速率（`crates/tui/src/app.rs`）
- 新增 `FRAME_MS=32`（~30 FPS）ticker + `dirty`/`render_pending` 双标志：事件立即处理但仅当两者皆真时重绘。`MissedTickBehavior::Skip` 防止 stall 后追赶爆发。
- 事件接收改为先 `try_recv` 排空全部再批量处理，把 token 突发合并为一次重绘。

### TUI：复制可见反馈（`crates/tui/src/selection.rs`、`crates/tui/src/app.rs`、`crates/tui/src/render.rs`）
- 新增 `CopyReport { lines, chars, osc52, local_tool }` + `status_message()`；`copy_to_clipboard`/`finish_copy` 返回报告。
- `app.rs` 鼠标拖拽复制后用 `copy_status: Option<(String, Instant)>`（2s 过期，基于 `Instant` 因为 anim_tick 仅 running 时推进）显示绿色 chip；按键时清除。
- `render.rs` 提取 `render_status_chip` 共享渲染（mode-flash + copy-status），chip 宽度改用 `str_width`（emoji 安全，修复宽度 chip 裁剪第二个 emoji）。

### TUI：显示宽度补全 + 截断按显示列（`crates/tui/src/composer.rs`、`crates/tui/src/chat.rs`、`crates/tui/src/task.rs`）
- `char_width` 扩展：零宽（组合标记、ZWSP/ZWNJ/ZWJ、变体选择符、BOM）+ 宽 emoji 范围（⌚⏩☔✂⭐等）+ CJK 扩展 B（plane 2）。
- `short()` / `short_preview()` 改为按**显示宽度**截断（`truncate_to_width`/`str_width`），不再按字符数——修复 CJK/emoji 截断越界。

### TUI：`/task` 切回始终从 store 重放（`crates/tui/src/app.rs`）
- 切回 Resume 会话时始终 `replay_into_chat` 从 store 重建 chat，而非复用可能过期的缓存快照（后台子代理可能在会话休眠期间完成）。

### TUI：输入线程 poll 失败静默退出（`crates/tui/src/input.rs`）、键位说明补充（`crates/tui/src/keybind.rs`）
- `input.rs` poll 失败改为静默 `break`，不再 `eprintln!`——alt-screen 激活时写 stderr 会破坏显示。
- `keybind.rs` 补充 SHIFT+drag 终端原生选择说明。

### 清理
- 删除 dead code：`app.rs` 的 `local_queue`/`VecDeque` 声明 + TurnDone 永不触发的 pop 块。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| plan 重定向：写文件被拦 | `file_write_redirects_blocked` | `crates/session/src/bash_guard.rs` |
| plan 重定向：只读放行 | `devnull_and_fd_merge_redirects_allowed` | `crates/session/src/bash_guard.rs` |
| plan 重定向：复合命令旁路 | `redirect_bypass_in_compound_blocked` | `crates/session/src/bash_guard.rs` |
| plan /dev/null 集成 | `plan_mode_allows_devnull_redirect` | `crates/session/tests/bash_guard_plan_mode.rs` |
| 压缩：工具轮次边界 | `split_index_assistant_after_tool_is_turn_boundary` | `crates/session/src/compaction.rs` |
| 压缩：多用户不变 | `split_index_multi_user_unchanged` | `crates/session/src/compaction.rs` |
| 压缩：轮次不足返 0 | `split_index_returns_zero_when_too_few_turns` | `crates/session/src/compaction.rs` |
| 压缩：混合边界 | `split_index_mixed_user_and_tool_turns` | `crates/session/src/compaction.rs` |
| 压缩：工具密集会话触发 | `compaction_fires_in_tool_intensive_single_user_session` | `crates/session/tests/compaction_and_model.rs` |
| skill 触发消息 | `skill_only_empty_prompt_starts_turn_with_skill_in_system_prompt` | `crates/session/tests/skill_mid_run.rs` |
| skill 触发记录 | `skill_only_empty_prompt_records_user_trigger_message` | `crates/session/tests/skill_mid_run.rs` |
| cancel 解阻塞挂起流 | `cancel_unblocks_a_hung_stream_promptly` | `crates/session/tests/quit_while_running.rs` |
| 子代理×交错思维(4) | `subagent_reasoning_persisted_on_child_tool_call_turn` 等 | `crates/session/tests/subagent_interleaved_thinking.rs` |
| wrap_rows 词边界 | `wrap_rows_breaks_at_word_boundary` 等(11) | `crates/tui/src/composer.rs` |
| 光标跨软换行 | `move_cursor_vertical_crosses_soft_wrap` 等 | `crates/tui/src/composer.rs` |
| char_width 零宽/emoji | `char_width_zero_width_combining_and_joiners` 等(2) | `crates/tui/src/composer.rs` |
| 渲染×光标对齐 | `composer_word_wrap_renders_and_cursor_aligns` | `crates/tui/src/render_tests.rs` |
| 状态 chip emoji 宽度 | `status_chip_width_accounts_for_wide_emoji` | `crates/tui/src/render_tests.rs` |
| short 按显示宽度截断 | `short_truncates_by_display_width_not_char_count` | `crates/tui/src/chat_tests.rs` |
| 合成消息重放跳过 | `replay_skips_synthetic_user_messages` | `crates/tui/src/session_ui.rs` |
| CopyReport 反馈(4) | `copy_report_status_with_local_tool` 等 | `crates/tui/src/selection.rs` |
| /task 切回刷新状态 | `replay_refreshes_status_after_subagent_completes` | `crates/tui/tests/subagent_replay.rs` |

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 编译：`cargo build --workspace` → 干净
- 行数：`app.rs` 852、`composer.rs` 762、`render.rs` 780、`runner.rs` 803、`bash_guard.rs` 561、`compaction.rs` 287、`selection.rs` 496（均 ≤ 800）

## Impact Surface
- **用户可感知**：Ctrl+D 不再卡死终端；高 token 速率下 CPU 占用下降（~30 FPS 封顶）；复制后有绿色反馈 chip；plan 模式可跑 `2>/dev/null`/`2>&1` 的只读命令；CJK/emoji 输入光标对齐与截断正确。
- **会话行为**：单用户多工具会话现在会触发压缩（更长会话不再撑爆上下文）；skill-only 提交记录一个用户 turn。
- **不影响**：`Store` trait 边界、`ChatStream` 抽象、web SSE 接口、CLI headless 运行时；这些修复均在 session runner / TUI 渲染层内部。

## Related Docs
- [agents/session](../../agents/session/index.md)
- [agents/tui](../../agents/tui/index.md)
- [agents/core](../../agents/core/index.md)
