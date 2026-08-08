Commit: 05d4bdf110cd7bfa75492f8ea7eebbb7cdb4c662

# feat(tui): 底部状态栏 running 动效与运行时间互换位置

## 背景

底部状态栏在运行态按 `… <spinner> <status>  <duration>` 的顺序渲染：先是 running 动效
（braille spinner + 状态文本，如 `compacting conversation…` / `interrupted`），最后才是
任务累计耗时。用户希望将运行时间放到 running 动效之前，使布局更贴近"先时间、后动效"的
阅读顺序。

## 变更

- **`crates/tui/src/render.rs` `render_status`**：任务累计耗时（`task_ms > 0` 时渲染的
  `format_run_duration`，warn 色）从 spinner/status **之后**移到**之前**，顺序变为
  `… <duration>  <spinner> <status>`（其中 `<status>` 为真实会话状态，如
  `compacting conversation…`，无硬编码 "thinking" 标签）；注释由 "motion → time" 更新为
  "time → motion"。
- **`crates/tui/src/render_tests/status_bar.rs`**：`status_bar_shows_task_time` 断言由
  `time_pos > spin_pos`（耗时在动效后）反转为 `time_pos < spin_pos`（耗时在动效前）。

## 测试清单（crates/tui，全部为 unit）

| 测试 | 位置 |
| --- | --- |
| `status_bar::status_bar_shows_task_time`（耗时在 spinner 前 + warn 色） | `render_tests/status_bar.rs` |
| `status_bar::status_bar_running_shows_spinner_and_status`（运行态动效+状态文本不回归） | `render_tests/status_bar.rs` |
| `status_bar::status_bar_hides_task_time_when_zero`（0 耗时隐藏不回归） | `render_tests/status_bar.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- UI 定向：`status_bar_shows_task_time` 验证耗时位于 spinner 前且保持 warn 色；状态栏 8 项定向测试全部通过。
