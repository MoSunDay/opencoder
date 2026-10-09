Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# OpenCoder 执行容量恢复

FleetStore 按 ticket、execution、runtime 三重身份校验容量恢复事务。重复恢复只返回已完成状态，身份不一致或阶段不允许时拒绝修改。

Worker 启动时检查遗留的 running ticket；发现仍有本运行时持有的进程时拒绝释放容量。平台维护命令提供批量恢复入口，核验运行时配置、节点绑定、进程归属和主机数据库快照，保存恢复回执。

## 测试覆盖

| 功能 | 测试入口 | 文件 |
| --- | --- | --- |
| 恢复事务的身份、阶段和重复请求校验 | 内联测试 | `crates/store/src/fleet/handoff/capacity_tests.rs` |
| 启动恢复与进程归属校验 | 内联测试 | `crates/worker/src/runtime/capacity.rs` |
| 批量容量恢复与命令参数 | Python 单元测试 | `scripts/platform/maintenance_tests/topology/test_capacity_recovery.py` |

- 原隔离回归：`cargo test --workspace --locked --no-fail-fast -- --test-threads=1`，459 个测试套、5,755 个测试、0 失败。
- `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo build --workspace --locked`：通过。
