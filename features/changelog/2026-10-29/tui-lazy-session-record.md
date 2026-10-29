# TUI 打开即建 task 记录改为提交首条输入后延迟创建

一句话：本地 TUI 启动不再预先写入会话（task）记录行；记录行改在用户真正提交第一条输入时创建——普通提示词沿用会话层 `persist()` 的懒创建（带标题），`@capability` 一次性子任务派发作为首条输入时在派发时刻补建父会话行与 runtime 行。打开后只是查看、不输入的会话不再留下空 task 记录。

## 变更清单

- `crates/tui/src/app_bootstrap.rs`：删除启动时的 `ensure_runtime_row` 预写入调用、函数及其两条测试；启动路径对 store 保持零写入。
- `crates/tui/src/remote/subtask_parent.rs`（新文件）：`ensure_parent_runtime`，在 `dispatch`（`remote/subtasks.rs`）读取父会话 runtime 前调用——runtime 行缺失（即本派发为会话首条输入）时创建会话行（`INSERT OR IGNORE`）并写入该 agent 的默认 runtime；已存在则原样返回不覆盖。`dispatch` / `request_dispatch` 增加 `agent` 参数。
- `crates/tui/src/app_submit.rs`：`mention_dispatch` 透传 `agent_name` 到派发链路。
- Web / local CLI 前端已是「首条 prompt 才建会话」，无需改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 派发作为首条输入时才创建父会话行与默认 runtime（打开 TUI 本身不落记录） | `dispatch_as_first_input_creates_parent_row_and_runtime` | `crates/tui/src/remote/subtask_parent.rs` |
| 已持久化的父 runtime 原样返回，懒创建不覆盖已注册子任务/特殊 runtime | `existing_parent_runtime_is_returned_untouched` | `crates/tui/src/remote/subtask_parent.rs` |

## 定向回归

- `cargo clippy -p opencoder-tui --all-targets` → 0 警告。
- `cargo test -p opencoder-tui --lib remote::subtasks --lib remote::subtask_parent` → 8 通过（含懒创建 2 项）；`--lib app_bootstrap` → 2 通过。
- `cargo test -p opencoder-tui --test subtask_restore_floor --test bootstrap_agent_override --test agent_switch_persist` → 7 通过。
- 未运行全量测试。
