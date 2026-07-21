# Subagent cancel/resume-replay：中断可重放，而非失败

## 背景

此前 subagent 被中断（cancel）时，要么被标记为 Failed（自然错误语义），要么留下悬挂的 tool_use。
正确语义应是：**Cancelled = 被打断 / 可重放**，与 Failed（自然错误结果）区分开。中断时父侧
tool_use 保持悬挂（不记录 tool 消息），resume 或下一轮用户消息时把子 agent 重放到完成并回填结果。

## 设计要点

- **Cancelled ≠ Failed**：`SubagentStatus` 新增 `Cancelled`（外加 `#[serde(other)] Unknown` 兜底未知旧值）。
- **Cancel 流**：`run_subagent` 通过共享 `CancellationToken` 检测中断 → 调 `cancel_subagent_task` 标记任务 Cancelled，
  发 `SubagentEnd { cancelled: true }`，返回 `ToolOutput::err("cancelled")`。`run_loop` 在记录 tool 消息前
  检测 cancel 并 `break`，**完全不落盘 tool 消息** → tool_use 保持悬挂、可重放。
- **Resume 流**：`resume()` 先把卡住的 Running 标记为 Cancelled；悬挂对账时构造 `replayable` 集合
  （Running + Cancelled 的 task tool_use），这些不合成 error 占位。`run_with_registry` 在进入 `run_loop`
  前调用 `replay_cancelled_tasks()` 逐个重放被 cancel 的子 agent 并回填结果消息。
- **递归类型环**：`replay_child` 内的 `run_with_registry` 调用用 `Box::pin` 打破 async 递归类型环。

## 变更

### Store 层（`crates/store`，全绿 36 测试）
- **`src/types.rs`**：`SubagentStatus` 增 `Cancelled` + `#[serde(other)] Unknown`；`as_str`/`parse` 同步。
- **`src/store.rs`**：`Store` trait 增 `cancel_subagent_task`。
- **`src/libsql_store/{mod.rs,subagent_tasks.rs}`**：`cancel()` SQL 函数 + delegate。
- Mock Store impl（`tui/app_helpers_tests.rs`、`session/tests/event_sink_flusher.rs`、`session/src/event_sink.rs`）同步。

### Session 层（`crates/session`，全绿）
- **`src/runner.rs`**：`SubagentEnd` 增 `cancelled: bool`（`#[serde(default)]`，向后兼容）；
  `sse_data`/`from_sse` 同步；`run_with_registry` 进 loop 前调 `replay_cancelled_tasks`；
  `run_loop` cancel 时跳过 tool 消息记录并 break；`run_subagent` cancel 后早期返回。
- **`src/resume.rs`**：stuck-Running → Cancelled；悬挂对账构造 `replayable` 集合；新增
  `pub async fn replay_cancelled_tasks()`；`replay_child` 用 `Box::pin`。

### TUI 层
- **`src/chat.rs`**：`ChatBlock::Subagent` 增 `cancelled: bool`；render badge `⊘`/DarkGray/"cancelled"。
- **`src/session_ui.rs`**：`build_subagent_block` Cancelled 分支。
- **`src/app.rs`**：启动时 `replay_into_chat` 回放历史 transcript（修复 Issue 1：TUI 启动空白）。

### CLI
- `format_resume_summary` 已有 Cancelled 分支（预先到位）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 中断→resume→重放→回填核心流（Running 子 agent） | `resume_and_replay_continues_running_child_and_backfills_result` | `crates/session/tests/resume_replay.rs` |
| 多个被 cancel 的子 agent 重放合并为单条回填消息 | `resume_and_replay_replays_multiple_children_into_one_backfill_message` | `crates/session/tests/resume_replay.rs` |
| 已完成（Completed）子 agent 不重跑 | `resume_and_replay_leaves_completed_tasks_untouched` | `crates/session/tests/resume_replay.rs` |
| 无卡住任务时 resume 正常继续 | `resume_and_replay_no_running_tasks_just_resumes` | `crates/session/tests/resume_replay.rs` |
| stuck-Running 在 resume 时标记为 Cancelled | `resume_marks_stuck_running_subagent_as_cancelled` | `crates/session/tests/resume_reconcile.rs` |
| 悬挂 tool_use 合成 error 占位（非可重放） | `resume_synthesizes_error_result_for_dangling_tool_use` | `crates/session/tests/resume_reconcile.rs` |
| `SubagentStatus` 序列化往返（Cancelled + Unknown 兜底） | `subagent_status_parse_and_as_str` | `crates/store/tests/store_integration.rs` |

> 注：cancel 早返回分支（`CancellationToken` 触发 → `SubagentEnd{cancelled:true}` + `cancel_subagent_task` 落库）
> 目前经 resume_replay 的 Running→replay 路径间接覆盖，暂无聚焦该分支的专门单测（非阻塞，review 备注 G4）。

## 验证

> 工作树当前混入多项并发进行中的特性（skill-menu/model_menu 重构、cache_salt_menu、cli ts、prompt-file 等），
> workspace 测试总数与 clippy 状态随并发改动波动。下述为本特性代码的当次实跑验证（2026-07-22）。

- `cargo test -p opencoder-store` → **36 passed / 0 failed / 0 ignored**（含 `subagent_status_parse_and_as_str`
  Cancelled/Unknown 往返、delivery round-trip、perf）。
- `cargo test -p opencoder-session` → **69 passed / 0 failed / 0 ignored**：单测 + 全部 integration
  （resume_replay 4 / resume_reconcile 6 / cancel_reset / handoff_resume / cancel 等）。
- `cargo test --workspace`（当次快照）→ **766 passed / 0 failed / 0 ignored**；本特性相关测试全绿
  （总数含并发进行中的特性，随工作树状态波动）。
- `cargo build --workspace` → **Finished**（零错误；tui lib 正常编译）。
- clippy：本特性触改的 crate（store / session）`-D warnings` 零警告。`cargo clippy --workspace --all-targets
  -- -D warnings` 当次报 2 处错误，均在并发进行中的 skill-menu 重构文件
  （`crates/tui/src/key_handler.rs:27` `SetSkill` dead_code、`crates/tui/src/menu.rs:489` `unnecessary_filter_map`），
  **与本特性无关**，归属该重构（提交时由 submit 隔离排除）。
- 源码无遗留 TODO/FIXME/`println!`/`dbg!`/硬编码密钥。
- 文件行数：新文件 `resume.rs` 450、`resume_replay.rs` 387、`types.rs` 235（≤ 400/800）；`chat.rs` 800（达限）。
  `runner.rs` 1131 / `app.rs` 1116 超 800——为 HEAD 基线即存在的预存债务（c2c904b review 已 DEFER），
  本特性仅微增，非本轮引入。
