# TUI 渲染提速 3FPS + 状态栏精简（移除 steer/queue，ctx% 移入正文）

## 背景

此前 TUI 限制为 1 FPS（`FRAME_MS=1000`），token 流式到达时刷新过慢；状态栏同时承载 steer/queue 计数与 ctx% 指示器，信息密度过高。本次将渲染上限提到 3 FPS，并把 steer/queue 计数从状态栏移除（队列面板仍保留其显示），ctx% 指示器迁移到正文（body）保留底部行——状态栏回归「模型 | agent | 运行状态」单一职责。

> 2026-07-14 的 changelog 已将 `tui/src/render.rs`、`tui/src/render_tests.rs` 标为「另一在途任务」，本条即为其收口。

## 变更

### `crates/tui/src/app.rs`
- `FRAME_MS` 1000 → 333（1 FPS → 3 FPS），并同步更新三处注释。事件仍即时处理，仅重绘上限放宽。

### `crates/tui/src/render.rs`
- `render_status` 签名精简：移除 `steer_count` / `queue_count` / `used` / `limit` 四个参数及其渲染逻辑，状态栏不再显示 `↳steer:N`、`queue:N`、`ctx N%`。
- `render_body` 签名扩展：新增 `used: u64` / `limit: u64`，在正文保留底部行渲染 ctx% 用量条（带分档配色：≥85% Red / ≥60% Yellow / 否则 Green）。
- 唯一生产调用点（`render`）同步更新。

### `crates/tui/src/keybind.rs`
- 帮助文案整理：`Ctrl+W`（backward-kill-word）补入编辑区分组，移除冗余的 Up/Down/Left/Right 行（这些在其它分组已涵盖）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 状态栏不再含 steer/queue/ctx（负向回归守卫） | `status_bar_has_no_steer_queue_or_ctx` | `tui/src/render_tests.rs`（新增） |
| 正文保留行渲染 ctx% + 紧凑 token 计数 | `body_shows_ctx_percent` | `tui/src/render_tests.rs`（由 `status_bar_shows_ctx_percent` 迁移至 body 位置） |
| 高用量时正文 ctx% 配色为 Red | `body_ctx_red_at_high_usage` | `tui/src/render_tests.rs`（由 `status_bar_ctx_red_at_high_usage` 迁移至 body 位置） |

> 说明：原 `status_bar_shows_ctx_percent` / `status_bar_ctx_red_at_high_usage` 断言的是 ctx% 出现在状态栏——该行为随 ctx% 迁移到 body 而消失。本次**非删除测试修绿**：两条用例随行为迁移改测 `render_body`，断言内容（ctx 出现、紧凑计数 5K/200K、Red 配色）保持不变，仅断言对象从状态栏改为正文。

## Gate

> 以下为当次实跑结果（工作树仅含本次 4 个文件改动）。

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 550 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |
