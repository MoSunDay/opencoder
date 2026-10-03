Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 原生 DAG 维护保护与回归收尾

## 变更

- 维护预检和资源升级保护 Server 宿主源工作区：检查现有目录与实际运行用户的读取、遍历权限，拒绝与应用可写路径重叠，不创建源目录或改变所有者、权限、内容。
- SQLite 备份的检查、附加与恢复使用只读 immutable 连接，避免读取 WAL 模式备份时新增 sidecar 文件并破坏校验；不对活动数据库使用 immutable。
- 增加隔离维护验收，使用实际旧 Server 证明迁移前进程崩溃后回滚，以及迁移后失败、拒绝旧回滚和同候选重试。源目录身份、鉴权行摘要、备份与配置快照共同核验。
- Worker 最后持有者释放节点锁时显式解锁，避免其他 fork 子进程或复制描述符延长占用；关闭过程仍等待执行和后台任务完整析构。
- DAG 流程测试先等待异步子会话索引发布并核验类型与节点，再严格检查会话正文；没有宿主回退、兼容分支或放宽会话断言。
- 拆分 Control、Web、Worker、Host、NFS 与 TUI 的超限测试文件，保留原用例与断言；补齐格式检查。

## 功能与测试

| 功能 | 测试名或入口 | 文件 |
| --- | --- | --- |
| 源目录不存在或路径重叠时拒绝 | `test_missing_workspace_is_rejected_without_creation_or_service_changes`、`test_source_inside_service_state_is_rejected_before_any_upgrade_writes` | `scripts/platform/maintenance_tests/test_workspace.py` |
| 实际用户权限与失败重试保护 | `test_actual_service_user_must_be_able_to_read_and_traverse_workspace`、`test_upgrade_failure_and_retry_preserve_source_with_a_different_owner` | `scripts/platform/maintenance_tests/test_workspace.py` |
| 旧导出服务升级前只规划新挂载 | `test_native_mounts_are_planned_before_old_exporter_is_replaced` | `scripts/platform/maintenance_tests/test_preflight.py` |
| WAL 备份恢复与重复读取不改备份 | `test_wal_backups_remain_immutable_through_restore_and_retry` | `scripts/platform/maintenance_tests/test_archive.py` |
| 执行与后台 future 完整析构 | `shutdown_waits_for_execution_task_destructors`、`shutdown_waits_for_background_task_destructors` | `crates/worker/src/state/tests.rs` |
| fork 与复制描述符不能延长节点锁 | `dropping_worker_releases_node_lock_while_a_forked_child_keeps_its_descriptor`、`dropping_worker_releases_node_lock_despite_a_duplicated_descriptor` | `crates/worker/src/state/tests.rs` |
| Agent 步骤的异步索引与会话读取 | `dag_spec_dispatch_runs_binary_and_agent_steps_to_done` | `tests/dag_e2e/flow.rs` |
| 实际维护崩溃、回滚、迁移与重试 | `main.py --platform-bundle ... --old-bundle ...` | `scripts/acceptance/maintenance_release/main.py` |

## 验收与边界

- 固定源码后执行 `cargo test --locked --workspace --no-fail-fast`：436 个目标，5590 passed、0 failed、7 个原有手工环境跳过项；`cargo fmt --all -- --check`、全目标 Clippy 零警告及 `cargo build --locked --workspace` 均通过。
- Python 验收共 143 项：维护 31、滚动 59、信号 12、安装器 19、平滑演练单测 10、维护验收单测 12。SPA 116 个文件、908 项测试通过；SPA 漂移检查通过。
- 最终开发二进制与配套私有镜像通过两节点真实 runc、20 个基础样本、预期错误、超时、取消、动态实例、16 MiB 产物和原节点重启；观察 900.008 秒、81 个样本。源目录未改动，无残留挂载，服务退出码均为 0。
- 同一开发镜像的平滑切换与回滚通过：51 个持续提交、最大受理 1.862 秒，按 30 秒门槛验收；真实项目浏览器与 TUI 110×34、72×34 均通过。
- 维护验收使用当前控制器代码、实际旧 `33bccb1e` Server 和已有干净 `da29072` 夹具包，真实 schema 为 28→32；两个维护场景及清理通过，鉴权行、源目录、备份和配置快照不变。31→32 由 Store `catalog_maintenance`、`project_tags` 测试单独覆盖，不伪造真实演练的版本。
- 该维护夹具的旧 Host/Runtime 与独立 NFS 导出服务使用候选二进制；服务命令适配器不证明宿主 systemd 依赖调度，不能代替完整旧环境或当前正式候选包验收。
- 当前开发二进制保留真实 dirty 状态与未知 SPA 摘要，严格维护入口在启动服务前以 `invalid spa_sha256` 拒绝。未提交 Git、推送或部署；正式候选包仍需干净提交、按正式流程构建并再次验收，不能据此宣称可上线。

## 证据

- 全量日志：`/tmp/opencoder-implementation-final-workspace.log`；格式、Clippy、构建日志同前缀。
- 固定源码清单：`/tmp/opencoder-implementation-source-h4f42o9m.receipt.json`；开发镜像清单：`/root/.opencoder-implementation-final.k32c0ya5/build.json`，`release_bundle=false`。
- 原生演练：`/root/.cache/opencoder-e2e/20261001-implementation-final-native/evidence/result.json`。
- 平滑切换：`/root/opencoder-smooth-lbxvpdsr/result.json`；维护夹具：`/root/.cache/opencoder-e2e/mr-ohpyxfwa/result.json`；开发包拒绝：`/root/.cache/opencoder-e2e/mr-sddk5q3h/result.json`。

相关：[执行规则](../../../rules/04-dag-execution-contract.md)、[Server 索引](../../../agents/server/index.md)、[Worker 索引](../../../agents/worker/index.md)、[调度平台](../../agent-platform/index.md)、[维护验收说明](../../../scripts/acceptance/maintenance_release/README.md)。
