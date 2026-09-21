Commit: fc047704e4c583cb9e0c11293b3815916ce1659b

# 磁盘准入阈值降至 10%

节点曾在约 2 TiB 根分区仍有约 380 GiB 可用时，因磁盘空闲低于 20% 拒绝新的审查任务。按操作方授权，将磁盘块空闲准入阈值调整为 10%。inode 空闲阈值仍为 20%，容量读取失败、零容量及持久化冻结继续拒绝新执行。

判断仍由 `runtime/health.rs` 的纯函数完成，未增加环境变量、配置表或数据库字段。健康状态与所有新执行入口复用同一判断；既有执行的完成行为不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 磁盘 9% 拒绝、10% 和 19% 可用、inode 19% 拒绝 | `storage_requires_ten_percent_blocks_and_twenty_percent_inodes` | `crates/worker/src/runtime/health.rs` |
| 零 blocks / inodes 继续拒绝 | `unknown_storage_capacity_remains_unhealthy` | `crates/worker/src/runtime/health.rs` |
| 所有新执行入口及持久化冻结 | `storage_and_durable_freeze_reject_every_new_work_entry` | `crates/worker/tests/drain_health/main.rs` |
| 低容量下已有工作继续完成 | `low_storage_blocks_new_work_while_existing_work_finishes_naturally` | `crates/worker/tests/drain_health/main.rs` |

验证回执位于 `/root/workspace/artifacts/test-agent/2026-09-20/skill-driven-four-repo-review/deployment/storage-threshold/`。

- 定向容量测试：2 passed / 0 failed。
- 全量 Clippy：`cargo clippy --workspace --all-targets --jobs 8 -- -D warnings` → 零警告。
- 全量回归：`cargo test --workspace --jobs 8` → 5,530 passed / 0 failed，仓库原有 8 ignored；未新增跳过项。
- 构建：`cargo build --workspace --jobs 8` → 通过。
- SPA：`npm test` → 120 文件、892 passed / 0 failed。
- 产物一致性：`scripts/check-spa-drift.sh` → no drift。

首次全量测试发现共享 target 中的服务端二进制提交过旧，先执行 `cargo build --workspace --bins --jobs 8` 后重新运行完整回归，以上结果取自完整通过的一轮。初次高并发编译中止记录和失败日志保留，不计入通过结果。发布从包含并发配置、画布修复及本阈值调整的干净提交构建，实际生效以同目录发布回执为准。
