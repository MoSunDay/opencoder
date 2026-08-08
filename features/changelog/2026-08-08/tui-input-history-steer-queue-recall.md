Commit: 05d4bdf110cd7bfa75492f8ea7eebbb7cdb4c662

# feat(tui): Enter / Tab 提交内容可经方向键在 input 找回

## 背景

运行中按 **Enter**（→ `KeyAction::Steer`）或 **Tab**（→ `KeyAction::Queue`）提交的内容
只进入 steer/queue 面板，从未写进输入历史——之后按 ↑/↓ 方向键无法在 input 里找回复用。
只有 idle 提交（Enter，`KeyAction::Submit`）经 `push_user` 记录历史。

## 变更

- **`app_helpers.rs`**：新增 `push_history(history, hist_idx, text)` —— 仅把输入写进
  方向键历史并重置 `hist_idx`，**不**回显 transcript marker（steer/queue 面板已展示原文）。
  `push_user` 重构为 `push_history` + transcript 回显两步，行为不变。
- **`app.rs`**：`KeyAction::Steer`（运行中 Enter）与 `KeyAction::Queue`（运行中 Tab）
  分支在 admit 之后调用 `push_history(&mut history, &mut hist_idx, &text)`，与 Submit 路径
  对齐。Up 箭头从此能按新→旧召回最近一次 Enter/Tab 提交的内容。
- **行数**：app.rs 保持 799 行，app_helpers.rs 保持 781 行（均 ≤ 800）。
- **附带修复（working tree 既有编译/逻辑问题）**：
  - `render_tests/cursor.rs`：`composer::cursor_screen_position` 改为
    `crate::composer::cursor_screen_position`（原引用解析到本地测试子模块 `render_tests::composer`，导致 lib test 编译失败）。
  - `session/src/subagent_steer_gate.rs`：`mod tests` 提前闭合的 `}` 移到文件末尾（两个
    `#[tokio::test]` 悬在模块外导致编译失败）；`settle_idle` 决策分支补上 `state.closed`
    检查（force_close 后等待中的 settle_idle 必须醒返回 `Close`，此前只检查 epoch/
    reservations 导致一直阻塞）。

## 测试清单（crates/tui unit）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| push_history 记录文本并重置 hist_idx | `push_history_records_text_and_resets_hist_idx` | unit(app_helpers) |
| 多条记录按新→旧累积 | `push_history_accumulates_newest_last` | unit(app_helpers) |
| 空文本仍记录、不留 stale 光标 | `push_history_records_empty_text_without_stale_cursor` | unit(app_helpers) |
| push_user 重构后仍记录历史 + 回显 `user:` marker | `push_user_records_history_and_echoes_transcript` | unit(app_helpers) |
| push_history 后 ↑ 找回、↓ 清空（完整召回流） | `up_arrow_recalls_recorded_steer_or_queue_text` | unit(app) |
| 附带：subagent_steer_gate `settle_idle` closed 唤醒修复 | `force_close_wakes_an_already_waiting_settle_idle` 等 6 个 | unit(session) |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（冻结工作树实跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`app.rs` 799 / `app_helpers.rs` 781（均 ≤ 800）。
