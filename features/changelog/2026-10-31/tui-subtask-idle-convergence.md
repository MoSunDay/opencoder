# TUI 子任务 idle 收敛映射为 done

一句话：`@` 派发的 agent/operator 子任务在服务端成功后收敛到 `idle`（worker 不上报 `done`），TUI watcher 新增本地映射把 `idle` 记作 `done`，让 `[sub n/m]` 徽标推进、token 折叠与 Ctrl+Enter 追加解锁，不再永久轮询。

## 变更清单

- `crates/tui/src/subtask_registry.rs`：新增纯函数 `settle_status(ExecutionStatus) -> Option<&'static str>`——pending/running/cancelling/interrupted 返回 `None`（仍可能推进），`idle` 映射为 `"done"`，done/error/cancelled 原样返回 `as_str()`；`is_terminal` 不变。
- `crates/tui/src/subtask_watcher.rs`：轮询落定判断由 `status.terminal()` 改为 `subtask_registry::settle_status(status)`，命中后照旧拉取 `/result` 摘要与 `/messages` 用量并按映射后的状态串发送 `UiEvent::SubtaskUpdate`；未命中则回到下一轮轮询，传输错误/MAX_FAILED_ROUNDS 行为不变。
- 仅改 `opencode-tui`，worker/core 协议与控制面不动（worker 侧 journal/brain 已把 `Idle` 视为完成，本变更对齐该先例）。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| settle_status 全 8 个 ExecutionStatus 变体映射（idle→done、终态→as_str、其余→None） | `settle_status_maps_server_lifecycle_to_registry_status` | `crates/tui/src/subtask_registry.rs` |
| `/index` 返回 `idle` 时 watcher 落定为 `done`，摘要/tokens 折叠进父注册表与聊天计数 | `watcher_maps_idle_agent_convergence_to_done` | `crates/tui/src/remote/tests/subtasks.rs` |

## 定向回归

- `cargo test -p opencoder-tui --lib subtask_registry` → 8 通过 / 0 失败（含新增 1 项）。
- `cargo test -p opencoder-tui --lib subtask_watcher` → 2 通过 / 0 失败。
- `cargo test -p opencoder-tui --lib remote::tests::subtasks` → 3 通过 / 0 失败（含新增 1 项）。

## 配置与依赖

- 无新依赖、无配置变化；`opencoder-tui` 新增对 `opencoder-core::fleet::ExecutionStatus` 的既有类型引用。

## 回滚步骤

- 还原上述两个源文件并删除两条新增测试即回滚；注册表数据无迁移（idle 落定只写入既有 `status` 字符串字段）。

## 遗留

- 无。
