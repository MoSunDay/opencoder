# TUI `/fork` 斜杠命令：从已有会话复制上下文创建新任务

## 背景

CLI 与 Web 已有 fork 能力（复制 session 的 meta + messages 到新 id），但实现
各自内联、行为重复。TUI 缺少 fork 入口，用户想基于旧对话上下文开新任务时只能
手动重建。本次新增 `/fork` 命令：弹出与 `/task` 同款会话选择面板（fork 模式），
选中会话后将其上下文复制进全新会话并立即切换到该会话。

## 变更

### 1. fork 逻辑收敛为共享实现（session crate）

- **`crates/session/src/fork.rs`（新）**：`fork_session(store, parent_id) -> Result<String>`
  —— 复制 meta + messages 到新 id，原 session 零修改；重置 `task_type`（落库为
  `parent` 默认）、清空 `summary_images`、`title` 追加 ` (fork)`、时间戳刷新。
  CLI 专属的 `eprintln!` 提示移出，由调用方自定输出。
- **`crates/cli/src/run.rs`**：删除本地重复实现，`--fork` 走共享函数，fork 提示
  `eprintln!` 保留在调用点。`cli/tests/fork_session.rs` 改从 `opencoder_session::fork`
  导入。
- **`crates/web/src/api_ops.rs`**：`POST /api/sessions/:id/fork` 改为调共享实现；
  session 不存在仍返回 404（handler 层先行判定），其余错误 500。

### 2. TUI 接线（command → picker → switch）

- **`crates/tui/src/command.rs`**：`COMMANDS` 注册 `/fork`；`SlashAction` 新增
  `Fork` variant；`parse`/`dispatch` 增加 `"fork" | "fk"` 映射。
- **`crates/tui/src/task.rs`**：`TaskPicker` 新增 `mode: PickerMode { Switch, Fork }`；
  `TaskPick` 新增 `Fork(String)`。`new_fork(sessions, current)` 构造 fork 模式：
  - `selection()`：fork 模式将选中行直接映射为 `Fork(id)`（无 "+ New task" 行）；
  - `row_count()`：fork 模式 = sessions 行数（无 "+ New task" / "Clear all"）；
  - `clear_row_index()`：fork 模式恒 `None` → 两段式 Clear-all 确认不可能误触；
  - render：fork 模式跳过 "+ New task" 行、行索引偏移 0、标题为
    ` Fork (↑/↓ select, Enter=fork context, Esc=cancel) `。
- **`crates/tui/src/app_loop.rs`**：`dispatch_command` 新增 `Fork` arm —— 与 `/task`
  同款父会话列表，但用 `TaskPicker::new_fork` 打开。
- **`crates/tui/src/app_task.rs`**：`switch_session` 新增 `Fork` arm ——
  `fork_session` 复制 → `resume_and_replay` 重建 SessionState（带拷贝的消息）；
  chat 重建与 `resumed_messages` 归入 Resume 同路径，切换后直接显示复制的上下文。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| fork 拷贝消息 + 重置 task_type/summary_images + title 后缀 | `fork_copies_messages_and_resets_meta` | `session/src/fork.rs` |
| fork 不存在的 session 报错 | `fork_nonexistent_session_errors` | `session/src/fork.rs` |
| fork 模式无 +New/Clear-all 行 | `fork_mode_has_no_new_or_clear_rows` | `tui/src/task.rs` |
| fork 模式 selection 映射 / 环绕 | `fork_mode_selection_returns_fork_ids` | `tui/src/task.rs` |
| fork 模式 Enter 返回 Pick(Fork) 并关闭 | `fork_mode_enter_returns_pick_and_closes` | `tui/src/task.rs` |
| fork 模式渲染标题 + 隐藏辅助行 | `fork_mode_render_shows_fork_title_and_hides_aux_rows` | `tui/src/task.rs` |
| fork 模式空列表 Enter 返回 Idle | `fork_mode_empty_sessions_enter_returns_idle` | `tui/src/task.rs` |

| `/fork` 命令 parse（别名/无斜杠/trim） | `parse_fork` | `tui/src/command.rs` |
| `/fork` dispatch（含别名拒绝） | `dispatch_fork` | `tui/src/command.rs` |
| 菜单输入 fork + Enter 派发 Fork 并关闭 | `enter_on_fork_dispatches` | `tui/src/command.rs` |

回归沿用既有测试面：`cli/tests/fork_session.rs`（3 项，改共享导入后仍全绿）、
`web/tests/web_api_ops.rs`（fork 404/拷贝/title 3 项）。

- 全量回归：`cargo test --workspace` → **2023 passed / 0 failed**
- session lib：`cargo test -p opencoder-session --lib` → **257 passed / 0 failed**
- TUI lib：`cargo test -p opencoder-tui --lib` → **1017 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误

## 影响面

- `agents/session/index.md` / `agents/web/index.md`：fork 实现位置描述已同步为共享模块。
- `features/index.md`：slash 命令清单补充 `/fork`。
- 兼容性：CLI `--fork` 与 Web fork 端点的行为/输出不变（提示语、404/500 语义一致）。
