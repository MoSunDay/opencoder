Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 原生 DAG 与共享容器工作区

## 变更

- DAG 的隔离边界是整次运行：一个节点、一个 runc 容器，Binary、Agent 和动态实例都在其中执行；不增加步骤级容器或宿主回退。
- Server 源工作区通过只读 NFS 导出，节点用本地 OverlayFS 写层；源路径、权限和内容不改动，不整目录复制。补齐只读 Linux ACL 查询以支持实际内核写时复制。
- 受理固定资源版本、依赖、配置和 UTC 日期目录。恢复只用原节点的固定数据；Worker 与 Project 丢失驱动时先清理遗留容器和挂载，再更新终态。
- 输出有界，产物只归档声明文件并流式校验完整性；Agent、知识库和私有文件使用独立容器挂载，不占用步骤名称，系统不自动归档私有目录。
- 控制面和独立资源服务共用二进制管理接口；支持大文件上传，转发保留查询、字节和响应元数据。每个 Runtime 使用配套私有镜像和私有挂载命名空间。
- 运行器必须与节点完整构建信息一致。持久规则写入 `rules/04-dag-execution-contract.md`，根 `AGENTS.md` 要求修改 DAG 前读取。
- 按用户明确的 30 秒门槛验收平滑切换，删除原先 1 秒门槛；覆盖恰好 30 秒通过与超限失败。

## 切换边界


## 功能与测试

| 功能 | 测试名或验收入口 | 文件 |
| --- | --- | --- |
| 严格协议与参数边界 | `native_spec_preserves_argument_boundaries_and_pins`、`invalid_types_fields_and_string_arguments_are_rejected` | `crates/dag/src/spec.rs` |
| 原子版本发布与并发 | `versions_are_monotonic_across_rollback`、`concurrent_creates_publish_exactly_one_version` | `crates/dag-binary/tests/write.rs` |
| 拒绝混用镜像运行器 | `rootfs_rejects_missing_mixed_version_and_linked_runners_before_admission` | `crates/dag-runtime/tests/preflight.rs` |
| 资源变化后仍用固定版本 | `accepted_binary_version_survives_publish_rollback_and_pool_removal` | `crates/dag-runtime/tests/resources.rs` |
| 依赖精确快照与特殊文件拒绝 | `selected_snapshot_pins_only_current_dependencies_and_detects_mutation`、`fifo_and_symlink_metadata_fail_without_publishing_or_waiting` | `crates/agents/tests/snapshot.rs` |
| 只读 ACL 查询与拒绝写入 | `readonly_acl_service_returns_file_permissions_and_rejects_all_writes` | `crates/agents/tests/nfs_acl.rs` |
| 共享容器内完整 Agent 工具调用 | `agent_tools_reasoning_and_messages_survive_the_shared_container` | `crates/dag-runtime/tests/run_loop/native_session.rs` |
| 二进制日志与取消落盘 | `native_step_output_is_mirrored_to_the_run_session`、`cancel_drain_persists_inflight_step_artifacts_and_frames` | `crates/dag-runtime/tests/run_loop/` |
| Project 配置固定与终态前清理 | `dag_acceptance_pins_config_and_lost_driver_cleanup_precedes_terminal_status` | `crates/project/src/service_tests/dag_recovery.rs` |
| 声明产物与 256 MiB 有界流式读取 | `declared_report_and_nested_evidence_require_integrity_and_stay_in_the_step`、`streams_256_mib_artifact_with_bounded_frames_and_memory` | `crates/worker/tests/artifact_stream.rs` |
| 私有文件与恢复隔离 | `private_dag_files_are_pinned_durable_and_absent_from_public_readback` | `crates/worker/tests/private_files.rs` |
| FIFO 准入不阻塞、不接单 | `fifo_resource_is_rejected_without_waiting_for_a_writer_or_accepting_execution` | `crates/worker/tests/resource_admission/main.rs` |
| 资源服务管理与代理 | `resource_service_publishes_native_versions_to_its_exported_pool` | `crates/control/tests/resource_service_native.rs` |
| Runtime 私有镜像和挂载 | `test_only_runtime_units_create_private_mounts_for_native_dags`、`test_corrupt_publication_fails_closed` | `scripts/platform/rolling_tests/test_native.py` |
| 30 秒边界与公共入口独立校验 | `test_continuity_accepts_thirty_seconds_and_rejects_only_over_budget`、`test_public_readiness_measures_availability_independently_of_tail_latency` | `scripts/acceptance/smooth_release/tests/test_live.py` |
| 两节点真实 COW、动态、重启与版本恢复 | `--observe-seconds 900` | `scripts/acceptance/runc_scheduling/main.py` |
| 三版并存、回滚、重放、进程与调度连续性 | 隔离进程发布演练 | `scripts/acceptance/smooth_release/main.py` |

## 验收结果

- `cargo test --workspace`：436 个套件，5614 passed、0 failed、8 ignored；保留现有需要手工环境的跳过项。`cargo clippy --workspace --all-targets -- -D warnings` 零警告；工作区二进制构建通过。
- SPA：116 个文件、908 项测试通过；实际浏览器验证静态与动态 DAG、日志、下载、刷新回放和 390px 视口。滚动发布测试 58 项、安装器 19 项、平滑验收单测 10 项通过。
- 真实 runc：两个节点、20 个基础样本，覆盖预期失败、超时、取消、三个动态实例、16 MiB 归档和原节点重启。冷启动 P95 为 0.685 秒；固定镜像后观察 900 秒、80 个样本。源目录保持原样，无残留挂载，三个服务正常退出。
- 发布演练：50 个持续请求、零失败；最大受理 2.592 秒、受理间隔 2.692 秒、调度间隔 2.93 秒，均满足 30 秒门槛。既有 TODO 与原生进程、三版归属、回滚和历史读取均通过。

当前能力见 [DAG 工作流](../../dag/index.md)、[运行时索引](../../../agents/dag-runtime/index.md)、[执行规则](../../../rules/04-dag-execution-contract.md)和[发布说明](../../../docs/smooth-release.md)。
