Commit: (working-tree, pre-initial-commit)

# fix(tui/session): 五项 gap 修复 — 图像数据安全、工具输出上限、技能触发、尺寸去抖

## 背景

健康度审查发现 5 项 TUI/Session 行为缺陷：图像在 store 写入失败时静默丢失；工具输出无限累积导致内存膨胀与渲染卡顿；图片+技能组合提交时触发消息被吞；尺寸 0×0 瞬态事件触发伪重绘。以下逐项修复，每项附独立测试。

## 变更

### Gap 1: 技能纯提交运行中分支的图像安全
- **问题**：Submit handler 的 `else`（running）分支使用 `drain_pending_images`，在检查 `store.admit_input` 成功之前就清空了 `pending_images`。若 store 写入失败，图像被静默丢弃。
- **修复**：改用 `snapshot_image_uris`（非破坏性快照），仅在 `store.admit_input` 返回 `Ok` 后手动 `pending_images.clear()`。Steer、Queue、Queue-skill 四个分支同步处理。
- **文件**：`crates/tui/src/app.rs`（4 处 call site）、`crates/tui/src/app_helpers.rs`（新 `snapshot_image_uris`）

### Gap 2: 工具输出行数无上限
- **问题**：`ToolEnd` 事件捕获全部输出行，单次工具结果可达数千行，导致 `ChatView` 内存膨胀、每次刷新 `flatten_with` 成本线性增长。
- **修复**：新增常量 `TOOL_OUTPUT_LINES = 200`（`chat_types.rs`），在 `chat.rs` 的 `apply()` 和 `session_ui.rs` 的 `replay_one()` 两处 `.take(TOOL_OUTPUT_LINES)` 截断。折叠模式不受影响（仅渲染 header）。
- **文件**：`crates/tui/src/chat_types.rs`、`crates/tui/src/chat.rs`、`crates/tui/src/session_ui.rs`

### Gap 3: 图片+技能组合提交丢失触发消息
- **问题**：`run_with_registry` 中，记录用户消息（`has_text || has_images`）与注入技能触发消息（`has_skill && !has_text`）原为 `if/else if` 关系。图片+技能组合（空文本 + 图片 + 活跃技能）走入 `if` 分支记录图片，但 `else if` 被跳过——技能触发消息未注入，模型看不到技能激活。
- **修复**：拆分为两个独立 `if`：先 `if has_text || has_images` 记录用户消息，再 `if has_skill && !has_text` 注入触发消息。两者不再互斥。
- **文件**：`crates/session/src/runner/mod.rs`

### Gap 4: `snapshot_image_uris` 不破坏 pending 缓冲区
- **问题**：原 `drain_pending_images` 同时返回 URIs 并清空缓冲区——不可分割。若在 store 写入前调用，失败后图像无法恢复。
- **修复**：新增 `snapshot_image_uris(&[(String, String)]) -> Vec<String>` 纯函数，仅读取不修改。调用方在成功路径手动 `.clear()`。原 `drain_pending_images` 保留（`#[cfg(test)]`）供既有测试使用。
- **文件**：`crates/tui/src/app_helpers.rs`

### Gap 5: 尺寸 0×0 瞬态事件去抖
- **问题**：`size_changed` 对 0×0 维度返回 `true`（视为变化），导致 minimize/detach 瞬态事件触发伪 resize + 全量重绘。`on_resize_event` 未同步 `last_size`，导致下一帧 `poll_idle_resize` 冗余触发。
- **修复**：`size_changed` 对 `cur.0 == 0 || cur.1 == 0` 返回 `false`。`on_resize_event` 新增 `&mut Option<(u16, u16)>` 参数，resize 后将新尺寸写回 `last_size`。
- **文件**：`crates/tui/src/resize.rs`（`size_changed`、`on_resize_event`、`poll_idle_resize`）、`crates/tui/src/app.rs`（call site 透传 `last_size`）

## 测试

| # | 测试名 | 文件 | 验证点 |
|---|--------|------|--------|
| 1 | `snapshot_image_uris_returns_uris_without_clearing` | `app_helpers_tests/mod.rs` | 快照返回所有 URI，缓冲区不被清空 |
| 2 | `snapshot_image_uris_empty_yields_empty` | `app_helpers_tests/mod.rs` | 空缓冲区快照返回空 Vec |
| 3 | `skill_only_submit_while_running_drains_images_via_queue` | `app_helpers_tests/mod.rs` | 技能纯提交+运行中：snapshot→admit→clear 序列，store 存储内容正确 |
| 4 | `tool_output_truncated_at_limit` | `chat_tests/tool_collapse.rs` | 5000 行输出截断至 TOOL_OUTPUT_LINES(200)，折叠/展开均受控 |
| 5 | `image_only_turn_with_skill_records_both_user_image_and_trigger` | `session/tests/skill_mid_run.rs` | 图片+技能：user 图片消息 + synthetic 触发消息均记录，assistant 响应存在 |
| 6 | `size_changed_false_for_zero_dimensions` | `app_tests.rs` | 0×0 返回 false（不触发伪 resize） |

## 结构性调整

为满足文件行数 ≤ 800 行规则，拆出两个新模块（纯提取，无行为变更）：

- **`crates/tui/src/app_bootstrap.rs`**（118 行）：从 `app.rs` 提取 `run()` 启动流程（配置加载、session resume/create、终端初始化），`app.rs` 从 872→779 行。
- **`crates/tui/src/resize.rs`**（52 行）：从 `app_helpers.rs` 提取 `size_changed`/`on_resize_event`/`poll_idle_resize`，`app_helpers.rs` 从 817→772 行。

## 验证

全量验证（HEAD `b6f830e` + 本轮工作树 diff）：

| 检查 | 结果 |
|------|------|
| `cargo test --workspace` | PASS — **1267 passed; 0 failed; 0 ignored** |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
| `cargo build --workspace` | PASS — 0 errors |
