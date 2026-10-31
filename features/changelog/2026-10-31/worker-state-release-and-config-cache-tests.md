# worker 执行态释放与配置缓存的行为锁定测试

一句话：为存量 worker 执行态释放与配置缓存路径补直接单测——终态释放只回收无引用的 lifecycle/preparation gate 与 maintenance 注册（被持有的 gate 保留）；`configuration_shared` 在输入文件未变时复用同一份 Arc，harness 作用域内与项目配置文件变化后重新加载。

## 变更清单

- `crates/worker/src/state/tests.rs:260` 新增 `terminal_release_collects_unused_gates_and_maintenance_registrations`。
- `crates/worker/src/state/tests.rs:300` 新增 `configuration_cache_reuses_the_load_until_input_files_change`。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 终态释放回收孤儿 gate / preparation gate / maintenance 注册，被持有的 in-use gate 保留 | `terminal_release_collects_unused_gates_and_maintenance_registrations` | `crates/worker/src/state/tests.rs` |
| 配置缓存按输入文件变化失效：未变复用 Arc、harness 作用域强制重载、覆盖 candidate 重写后失效 | `configuration_cache_reuses_the_load_until_input_files_change` | `crates/worker/src/state/tests.rs` |

## 定向回归

- `cargo clippy -p opencoder-worker --all-targets` → 0 警告。
- `cargo test -p opencoder-worker --lib state::tests` → 9 通过 / 0 失败（含新增 2 项）。

## 配置与依赖

- 纯测试新增；被测实现（`state.rs` 的 `release_execution_state` / `configuration_shared` / `lifecycle_gate`、`maintenance_tools::install`、`state/config_cache.rs`）均为存量。

## 回滚步骤

- 删除两条测试即回滚；无生产代码变化。

## 遗留

- 无。
