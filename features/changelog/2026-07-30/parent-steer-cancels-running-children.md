# feat(session/web/tui): 父级 steer 中断时取消运行中的子代理

## 背景

会话派发子代理（subagent）后，父级的 `run_loop` 阻塞在 `run_subagent` 内等待子代理
整轮完成。此时若用户发起 steer（Web `POST /prompt` 或 TUI `>` 中断），父级无法在子代理
跑完前吸收该 steer——子代理一轮可能跑数分钟，steer 被长时间挂起；或父级被整体 kill，
丢失进行中的上下文。

需要一个机制：父级 steer 时**只取消运行中的子代理**，让父级 `run_loop` 在下一个 turn
边界吸收 steer，而不结束父级自身的运行。

## 变更

### 1. `fire_child_cancels` helper（`crates/session/src/lib.rs`）

新增纯函数，对 `child_cancels` 注册表里的每个 `CancellationToken` 调用 `.cancel()`：

- 注册表为空时返回 `false`（无副作用）；
- 至少取消一个子代理时返回 `true`。

触发后，`run_subagent` 的子代理 `run_loop` 在循环顶部 cancel 检查处中断，返回
`err("cancelled")` 工具结果，父级继续自身 turn 并在下个边界吸收 steer。

### 2. `child_cancels` 注册表（`crates/session/src/lib.rs` + `runner/subagent.rs`）

`SessionState` 新增字段：

```rust
pub child_cancels: Arc<Mutex<HashMap<String, CancellationToken>>>
```

以 `task_id`（call_id）为键。`run_subagent` 派发子代理时，由 `child_token()` 派生的
取消令牌登记进该表；子代理结束后移除。注册表为可复位的 `Mutex<CancellationToken>`
语义（见 `SharedCancel`），锁仅在 clone-check-fire 期间短暂持有，绝不跨 `.await`。

### 3. Web 集成（`crates/web/src/handle.rs`）

`SessionHandle` 增加同名 `child_cancels` 字段，与 `SessionState` 共享同一 `Arc`
（drain 启动时 `session.child_cancels = handle.child_cancels.clone()`）。`POST /prompt`
的 steer 路径在 admit 后无条件 `fire_child_cancels`——无子代理在跑时为 no-op，有则取消，
使父级吸收新 steer。父级 `run_loop` 自身**不被结束**。

### 4. TUI 集成（`crates/tui/src/app.rs`）

运行中按 `>`（steer）时：优先 `fire_child_cancels`；仅当**无子代理注册**（返回 false）时
才回退到取消父级自身 token 的旧行为。即有子代理在跑时只打断子代理、保留父级运行。

## 影响

- 纯增量行为：无子代理在跑时与改动前完全一致（`fire_child_cancels` 返回 false / no-op）。
- 父级 `run_loop` 结构不变，子代理取消复用既有 cancel-token 检查路径。
- 不读写数据库，不改变持久化语义；被取消的子代理仍按既有路径记录 `SubagentEnd`。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| 空注册表返回 false，无副作用 | `fire_child_cancels_returns_false_on_empty_registry` | `crates/session/src/lib.rs`（unit） |
| 多 token 全部被取消并返回 true | `fire_child_cancels_cancels_all_registered_tokens` | `crates/session/src/lib.rs`（unit） |
| 父 steer 取消运行中的 bash("sleep 30") 子代理，父级 <10s 恢复，发出 `SubagentEnd{cancelled:true}` | `parent_steer_cancels_running_child` | `crates/session/tests/child_cancel.rs`（integration） |
| `SessionHandle` 构造携带新 `child_cancels` 字段（编译期守护） | `interrupt_cancels_running_drain_token` 等 | `crates/web/tests/web_contract.rs`（integration） |


## 验证

- `cargo test -p opencoder-session --test child_cancel` -> 1 passed（`parent_steer_cancels_running_child`）。
- `cargo test -p opencoder-session --lib fire_child_cancels` -> 2 passed（2 个 unit）。
- `cargo test --workspace --all-targets` -> 全绿，0 failed（测试总数因并发进行中的其它改动而浮动，本任务验证时快照约 1391）。
- `cargo clippy --workspace --all-targets -- -D warnings` -> 零警告。
