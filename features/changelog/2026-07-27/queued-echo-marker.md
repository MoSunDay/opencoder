Commit: (working-tree, pre-initial-commit)

# feat(tui): queued echo marker + app_loop_tests 分拆 + 既有重构编译修复

## 背景

当 idle 边界消费一条 queued follow-up（`SessionEvent::QueueConsumed`）时，UI 仅从
`queue_items` 镜像中静默删除该行，用户看不到消费时机。`SteerConsumed` 已有
`steer: {prompt}` 标记，queued 路径缺少对称的视觉反馈。

同时，既有 tool-collapse / render 重构（`render_hits.rs` 抽取、`ToolBtn` 类型、
`MouseHits.tool_btns` 字段、`collapse_all_thinking → collapse_all_collapsible` 重命名）
未同步更新测试代码，导致 `cargo test --workspace` 无法编译。另外
`app_loop_tests.rs` 因新增 queued echo 测试达 856 行，超出 800 行迭代上限。

## 变更

### queued echo 标记（`crates/tui/src/app_loop.rs`）

- **`fold_ui_events`**：在 `QueueConsumed` 分支中，消费前解析 `seq → prompt`，
  推送一条 `queued: {prompt}` 黄色加粗标记 + 空行，镜像 `SteerConsumed` 的
  `steer: {prompt}` 青色标记语义。

### app_loop_tests 分拆（`crates/tui/src/app_loop_tests/`）

- 将单文件 `app_loop_tests.rs`（856 行）拆为目录模块：
  `mod.rs`（658 行）+ `model_outcome_tests.rs`（211 行）。
- `HOME_TEST_LOCK` / `EnvGuard` 保留在 `mod.rs` 作为共享基础设施。
- `plan_edit_tests` / `session_only_tests` 的 `#[path]` 从 `"*.rs"` 改为 `"../*.rs"`。

### 既有重构编译修复

- **`render_tests.rs`**：`record_thinking_hits` → `hit_records::record_thinking_hits`（5 处）；
  `render_body` 调用补 `tool_btns: &mut Vec::new()` 参数（4 处）。
- **`app_helpers_tests/mouse_tests.rs`**：`MouseHits` 字面量补 `tool_btns: Vec::new()`（3 处）。
- **`render.rs`**：`ToolBtn` 加 `#[allow(dead_code)]`（与 `SubagentBtn` 一致）。
- **`app_helpers.rs`**：`collapse_view` elide needless lifetimes（clippy）。
- **`chat_tests.rs`**：`toggle_tool_at_expands_then_collapses` 的 output 从 `"hi"` 改为
  `"RESULT-42"`，避免与 header 中的 command 文本 `"echo hi"` 子串碰撞。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| queued echo 标记推送 + 条目删除 | `fold_queue_consumed_pushes_marker_and_drops_entry` | `app_loop_tests/mod.rs` |
| 未知 seq 的 QueueConsumed 为 no-op | `fold_queue_consumed_unknown_seq_is_noop` | `app_loop_tests/mod.rs` |

- 全量回归：`cargo test --workspace` → **1213 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished clean
- 行数：`app_loop_tests/mod.rs` 658、`app_loop_tests/model_outcome_tests.rs` 211（均 ≤ 800）

## Impact Surface

- TUI 用户在 queued follow-up 被消费时会看到 `queued: {prompt}` 黄色标记。
- 不影响：Store/ChatStream/session/LLM 边界；CLI/Web 契约；runner/drain 语义。
