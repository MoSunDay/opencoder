# TUI：双 Esc 硬中止同时清空待处理的 steer/queue 输入

## 背景

会话运行中（`running == true`）按一次 Esc 是「软清空」——清掉输入框草稿、收起帮助等，不打断模型。要真正中止一个运行中的 turn，需要双 Esc 硬中止：在 350ms 窗口内连按两次 Esc，触发 `KeyAction::Cancel`，cancel 当前 turn 的 `CancellationToken`。

但此前硬中止只取消了当前 turn，**没有处理已经入队但尚未消费的 steer/queue 输入**。这些 pending 行仍留在 `Store`（`session_inputs` 表）和 TUI 的两份内存镜像（`steer_items` / `queue_items`）里。后果：用户中止后若 resume 或下一轮 drain 启动，这些「已经不想要」的输入会重新冒出来被消费——中止并未真正清场。

需求：**双 Esc 硬中止时，连同所有待处理的 steer/queue 输入一起清掉**，让中止后状态干净、resume 不会复活幽灵输入。

## 变更

### `clear_pending_inputs`（`crates/tui/src/app_helpers.rs`）

`pub(crate) async fn clear_pending_inputs(store, steer_items, queue_items)`：

- 遍历两份内存镜像（steer + queue），对每个 `seq` 调用 `store.delete_input(seq)`。
- 清空两份镜像。
- `delete_input` 只删 `promoted_seq IS NULL` 的行（已被 runner 提升/消费的不会被误删），所以即便 runner 已经消费了其中一部分，对两份镜像全量 fan-out 也是安全的。

### `KeyAction::Cancel` 分支接线（`crates/tui/src/app.rs`）

`KeyAction::Cancel` 分支在 `cancel.cancel()` 之后追加调用 `clear_pending_inputs(store.as_ref(), &mut steer_items, &mut queue_items)`，把硬中止的「取消当前 turn」与「清空待处理输入」绑成一个原子动作。随后照常推 `[interrupted]` marker、`running = false`。

### 双 Esc 判定（`crates/tui/src/key_handler.rs`，既有逻辑）

`ESC_CANCEL_WINDOW_MS = 350`：运行中首次 Esc 记录时间戳、返回 `KeyAction::None`（软清空）；窗口内第二次 Esc 返回 `KeyAction::Cancel`（硬中止），并重置时间戳。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行中双 Esc 在窗口内产出 `KeyAction::Cancel` | `double_esc_while_running_cancels` | `tui/src/app_tests.rs` |
| `clear_pending_inputs` 删除 store 行并清空两份镜像 | `clear_pending_inputs_drops_store_rows_and_mirrors` | `tui/src/app_helpers.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 625 passed / 0 failed / 0 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |
