Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# TUI Server 任务恢复与维护升级验收

## 变化与范围

- `/agent` 的 Server Agent/Operator 任务复用既有聊天渲染，支持 Codex Operator；首轮崩溃恢复即使已有 session 行，也先补齐 Harness、环境、展示策略和 Server 模型设置，再投递输入。已启动线程与冻结 HOME 保持不变。
- 原生 DAG journal、项目 schema v32 使用发布数据格式 2。维护发布在启动候选 Server 前持久化迁移意图；启动回执丢失时仍禁止恢复旧库，必须向前修复。公共入口关闭期间只允许所需的节点通道和本机 Host 状态读取。
- 停服、备份后先升级独立资源服务，再安装新的二进制与工作区只读 NFS 挂载。旧资源服务缺少构建信息时，兼容发布在启动候选前拒绝它。
- 旧空闲 Runtime 保存最终库存并休眠；冻结或复开不唤醒它。实际请求唤醒时先同步当前准入模式。Host 固定关闭信号 future，停止重试使用本机 systemd 接受的 `--kill-who=main`。
- DAG 等待内核卸载完成才提交清理结果和释放容量；卸载失败保留归属供重试。日志测试等待独立 stdout/stderr 两路都到达，保留运行中、事件顺序和重放断言。
- 全量测试镜像包含三个运行器；休眠读取测试提供真实结构的库存接口。两项修正补齐测试准备，不改生产执行约定。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| TUI 能力选择、独立任务、恢复、Codex 包装器 | 原有功能映射见 [TUI Server 能力](tui-server-capabilities.md#功能与测试) | [TUI 真终端验收](../../../scripts/acceptance/tui_server.py) |
| 各初始化断点恢复、`@` 原文和首轮不重复 | `recovery_admits_once_from_every_pre_execution_fault_point`、`resume_after_completed_first_turn_does_not_call_the_model_again` | [initial_input_recovery.rs](../../../crates/worker/tests/initial_input_recovery.rs) |
| 半完成 Operator 会话保留 Server Codex profile、线程与 HOME | `operator_recovery_completes_each_initialization_stage_and_keeps_server_profile` | [operator_initialization_recovery.rs](../../../crates/worker/tests/operator_initialization_recovery.rs) |
| 新库版本在表结构 DDL 前拒绝且数据不变 | `newer_schema_is_rejected_before_creating_or_modifying_tables` | [schema_bootstrap.rs](../../../crates/store/tests/schema_bootstrap.rs) |
| 数据格式 1/2 不能滚动混用 | `native_project_release_requires_maintenance_from_previous_format` | [release.rs](../../../crates/core/src/fleet/release.rs) |
| 资源版本、认证、二进制发布/下载与中继 | `resource_service_publishes_native_versions_to_its_exported_pool` | [resource_service_native.rs](../../../crates/control/tests/resource_service_native.rs) |
| 迁移启动回执丢失后禁止旧库恢复 | `test_lost_schema_start_reply_forbids_old_restore_before_public_writes` | [test_flow.py](../../../scripts/platform/maintenance_tests/test_flow.py) |
| 阶段中断沿用同一备份与候选、配置安装时间固定 | `test_every_durable_stage_resumes_same_backup_and_candidate`、`test_pending_configuration_is_private_and_installed_only_from_fixed_snapshot` | [test_flow.py](../../../scripts/platform/maintenance_tests/test_flow.py)、[test_configuration.py](../../../scripts/platform/maintenance_tests/test_configuration.py) |
| 旧生产资源升级前仅计划新挂载、拒绝未知工作区选项 | `test_native_mounts_are_planned_before_old_exporter_is_replaced`、`test_workspace_export_rejects_fields_that_config_loader_would_ignore` | [test_preflight.py](../../../scripts/platform/maintenance_tests/test_preflight.py) |
| 旧库恢复限迁移启动前，认证表不写入 | `test_restore_reinstates_v31_projects_indexes_and_old_writes_without_auth_writes` | [test_archive.py](../../../scripts/platform/maintenance_tests/test_archive.py) |
| 旧资源服务在预热前拒绝 | `test_old_resource_service_is_rejected_before_candidate_or_service_changes` | [test_deployment.py](../../../scripts/platform/rolling_tests/test_deployment.py) |
| 停止信号重试与当前 systemd 参数 | `test_stop_repeats_graceful_signal_after_lost_first_delivery`、`test_systemctl_accepts_emitted_signal_arguments_without_sending_a_signal` | [test_flow.py](../../../scripts/platform/maintenance_tests/test_flow.py)、[test_signal.py](../../../scripts/platform/signal_tests/test_signal.py) |
| 准入变更不唤醒休眠版本，实际访问继承当前模式 | `admission_changes_leave_hibernated_runtimes_stopped`、`accessed_hibernated_runtime_inherits_current_admission_mode` | [hibernation.rs](../../../crates/agent/src/host/tests/hibernation.rs) |
| 慢卸载等待、错误时保留清理归属 | `cleanup_keeps_ownership_until_slow_unmount_finishes`、`failed_unmount_preserves_ownership_for_cleanup_retry` | [sandbox/run/mod.rs](../../../crates/dag-runtime/src/sandbox/run/mod.rs) |
| Agent/二进制运行中日志、序号重放 | `dag_agent_and_binary_logs_stream_before_completion_and_resume_by_sequence` | [dag_live_logs.rs](../../../crates/worker/tests/dag_live_logs.rs) |

## 冻结版本与全量结果

验证对象是完整工作区的独立冻结检出 `/tmp/opencoder-launch-candidate`，commit `da290729ab54d5aacf9e1153f46c427fbc18f9b3`，Git clean；包含当前原生 DAG 与项目 schema v32 改动。最终核对 2,158 个代码/依赖文件与共享工作区一致。它与前一篇记录的 schema v31 TUI 独立检出范围不同，测试数量不混算。

- `cargo test --locked --workspace --no-fail-fast`：436 个测试目标，**5618 passed / 0 failed / 8 ignored**，完整日志 `/tmp/opencoder-closure-workspace-da29072.log`。同一原生 DAG 基线前次完整运行是 5615 passed / 3 failed / 8 ignored；补齐镜像、库存夹具与日志等待后，本次完整重跑为零失败。
- 8 项既有忽略用例是 3 项手工 NFS、3 项手工 runc、1 项部署设备夹具、1 项 Chromium 验收；未增加忽略项。真实 NFS/runc 另有下述运行证据。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告，`/tmp/opencoder-closure-clippy-da29072.log`；工作区构建成功，`/tmp/opencoder-closure-build-da29072.log`。
- Python：维护 24、滚动 59、信号 12，共 **95 passed**；日志 `/tmp/opencoder-closure-{maintenance,rolling,signals}-da29072.log`。
- SPA：116 个测试文件、**908 passed**，`/tmp/opencoder-closure-spa.log`；源码及 dist 在最终构建期间无变化，摘要 `095ccb2cc83b2bc24a0146b6888f69ec4b253e95f2dad5bde0efa37d37225559`。
- 真 PTY：110×34、72×34 均通过，包含选择器、`@` 原文、Server 渲染、续聊、self 隔离、任务恢复、CLI Codex 恢复和退出断开；`/tmp/opencoder-closure-tui-pty-da29072.log`。
- 测试构建仅去掉开发调试符号，保留断言；4 个测试线程，私有可执行 tmpfs 保存临时测试数据，镜像包含 `dag-runner`、`agent-step-runner`、`agent-session-runner`。实际运行演练保留在物理磁盘，不能用 tmpfs 的性能代表生产。
- 新建代码文件 ≤400 行、修改代码文件 ≤800 行；维护目录 10 个 Python 文件，Worker 根目录 10 个 Rust 文件；`git diff --check` 通过。

## 真实运行证据

全部使用专属临时目录、私有挂载空间和隔离凭据，**未发布或修改生产环境**。

| 验收 | 结果与证据 |
| --- | --- |
| 停服维护、v31→v32、丢失启动回执及向前续跑 | PASS，18.50 秒；旧库恢复明确拒绝、同一候选/备份续跑、认证行不变、三个 NFS 源只读、私有及公共原生探针通过。`/root/.cache/opencoder-e2e/20260930-closure-maintenance-da29072/result.json` |
| 真实 systemd/Nginx/NFS/runc 发布及回滚 | PASS，三次发布、一次回滚、128 次连续提交；任务与 OCI/Shell 原进程连续，SSE 恢复 0.103 秒，最大受理 1.919 秒。`/tmp/opencoder-smooth-xohpauqt/result.json` |
| 两节点原生执行、重启、固定版本恢复及 15 分钟观察 | PASS，20 个执行样本，冷启动 P95 0.295 秒；观察 901.718 秒、85 个观察样本，源数据不变、无剩余挂载、三个进程均退出 0。`/root/.cache/opencoder-e2e/20260930-closure-runc-da29072/evidence/result.json` |

维护演练运行真实 Nginx、NFS、runc 和服务进程，生成的 unit 命令由私有适配器执行；该演练本身不冒充 systemd 验收，真实 systemd 由平滑发布演练单独覆盖。原生执行验收中的 4 个失败步骤和 1 个取消运行是预期故障用例。

发布包 `/tmp/opencoder-launch-bundle-da29072` 的 6 个二进制均核对摘要与完整构建信息；交接协议 1、数据格式 2、节点协议 10。完整验收回执 `/tmp/opencoder-closure-final.json` 保存版本、校验值、测试结果及证据路径。

## 相关文档

- [TUI 原始能力变更](tui-server-capabilities.md)、[Agent 调度平台](../../agent-platform/index.md)
- [Worker](../../../agents/worker/index.md)、[Agent Host](../../../agents/agent/index.md)、[DAG Runtime](../../../agents/dag-runtime/index.md)、[Server](../../../agents/server/index.md)
- [维护与发布操作](../../../docs/smooth-release.md)
