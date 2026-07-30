# 状态栏进度条改为基于压缩阈值而非最大上下文窗口

## 背景

TUI 状态栏中 `[agent]` 与 `ctx` 之间的进度条（`▰▰▰▱▱` 可视化计量条）此前以 `used / context_limit`（最大上下文窗口，默认 128K）为分母计算填充比例。但实际自动压缩（compaction）在 `context_threshold`（默认 80K）处触发，导致进度条到 ~62% 时就已开始压缩，用户无法直观判断"距离压缩还剩多少"。

## 变更

将进度条的分母从 `context_limit` 改为 `compaction.context_threshold`，使进度条在达到 100% 时恰好对应压缩触发点。

### 涉及文件

- **`crates/tui/src/render.rs`**：`render_status()` 的 `limit` 参数更名为 `compaction_threshold`；进度条百分比 `context_percent(used, compaction_threshold, ...)` 及文本 `ctx N% (used/threshold)` 均改为基于压缩阈值。注释同步更新。
- **`crates/tui/src/render.rs`**：`render()` 参数 `context_limit` → `compaction_threshold`，透传至 `render_status`。
- **`crates/tui/src/frame.rs`**：`render_frame()` 参数 `context_limit` → `compaction_threshold`。
- **`crates/tui/src/app.rs`**：`run_app()` 参数 `context_limit` → `compaction_threshold`。
- **`crates/tui/src/app_bootstrap.rs`**：初始值 `session.config.context_limit()` → `session.config.compaction.context_threshold`。
- **`crates/tui/src/app_loop.rs`**：`handle_model_outcome()` 参数及两处更新逻辑（`reloaded.compaction.context_threshold`、`config.compaction.context_threshold`）。
- **`crates/tui/src/app_loop_tests/model_outcome_tests.rs`**、**`app_loop_session_only_tests.rs`**：测试变量名同步更新。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| model_outcome 客户端构建失败推红标记 | `handle_model_outcome_client_build_failure_pushes_red_marker` | `app_loop_tests/model_outcome_tests.rs` |
| model_outcome endpoint 解析失败推红标记 | `handle_model_outcome_endpoint_resolve_failure_pushes_red_marker` | `app_loop_tests/model_outcome_tests.rs` |
| session-only 切换不写盘 | `handle_model_outcome_session_only_skips_disk_write` | `app_loop_session_only_tests.rs` |

- 全量回归：`cargo test --workspace` → 全绿（652+ tests passed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：所有文件均远低于 400/800 行限制

## Impact Surface

- 用户可感知：状态栏进度条现在反映"距离压缩还有多远"而非"上下文窗口使用率"，100% 即压缩触发点。
- 不影响：压缩逻辑本身（由 session runtime 控制）、CLI、Web、store 等边界。
- `CONTEXT_BASELINE`（4000 tokens）仍保留，确保小会话不显示误导性的非零百分比。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [agents/core](../../agents/core/index.md) — `CompactionConfig.context_threshold`
