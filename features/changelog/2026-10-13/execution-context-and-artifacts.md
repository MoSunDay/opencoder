Commit: 37ee51ce6274e0ade8eb9fc8a34da1a1b7e7aa57

# DAG 与 Brain 完整输入及文件交接

## 目标与边界

一次根 run 独立管理 context、执行索引和恢复记录，包含各层、返工与恢复。各能力保留自己的工作区，通过明确的输入、输出和文件引用交接；DAG 仍遵守单节点、单容器和只读源工作区约定。用户在 2026-10-13 明确接受此边界并要求实施。

## 实现

- DAG 静态二进制、Agent 和动态实例的只读 context 提供完整冻结 `inputs`、`run_id`；动态实例另有实际输入和编号。原有依赖结果、参数数组与 how 追加语义保留，恢复拒绝保存输入漂移。
- 运行器在启动容器前保存完整 `inputs.json`；恢复拒绝输入漂移或快照缺失，恢复原输入后可继续原实例及资源副本。动态测试按实际 `instance.input` 区分执行，完整 context 中其他输入不代表那些实例已启动。
- 执行绑定增加可选 `source: output | artifacts`，默认 output；文件引用增加可选实例 index，保留 execution、step、file、bytes 与 SHA-256。DAG 业务文件及动态实例进入索引，其他能力索引自己声明的文件。
- 叶子输出与文件引用合计限 16 KiB；较大索引与嵌套 Brain 使用带内容摘要的清单引用，不裁剪成功输出。文件仍通过所属执行节点的产物接口读取；Brain 托管的 Agent、Team、TODO 也可以下载文件。
- 普通和共享目录计划统一拒绝跨 run 及旧 activation 的执行绑定，允许最新失败证据作为整改输入。Worker 校验决策 context 中的文件执行归属，普通 annotate 不能修改自身索引归属。
- 新 DAG、根 Brain 和托管子执行要求 `execution_io_v1`，先升级 Worker 后启用 Server；历史绑定及无实例引用仍可读取，原 Runtime 的执行归属保留。
- 保留并验证此前工作树中的当前 Error 回执恢复修复，见 [恢复记录](brain-store-current-error-recovery.md)。本次没有数据库结构迁移。

## 功能与测试对应

| 验收项 | 测试名 | 文件 |
| --- | --- | --- |
| 命名输入、动态实例、跨节点与 Server 重启 | `named_inputs_dynamic_artifacts_cross_nodes_and_server_restart_preserve_sources` | [brain_io/main.rs](../../../crates/worker/tests/brain_io/main.rs) |
| 两个 run 调用同一能力时输入、输出与文件分开 | `concurrent_runs_keep_same_capability_inputs_outputs_and_files_separate` | [brain_io/main.rs](../../../crates/worker/tests/brain_io/main.rs) |
| 嵌套 Brain 清单实际下载、校验与传递 | `nested_brain_exports_a_verified_manifest_to_the_parent_consumer` | [brain_io/main.rs](../../../crates/worker/tests/brain_io/main.rs) |
| Team → TODO 的结构化结果及空索引 | `team_and_todo_transfer_structured_outputs_and_empty_artifact_indexes` | [adapters.rs](../../../crates/worker/tests/brain_io/adapters.rs) |
| 声明文件、身份绑定、未声明文件及篡改拒绝 | `declared_operator_file_is_bound_by_identity_and_tampering_is_rejected` | [artifacts.rs](../../../crates/worker/tests/brain_io/artifacts.rs) |
| `path`/`file` 声明及输出中的报告文件实际下载 | `report_archive_path_is_indexed_and_downloaded_with_the_declared_digest` | [artifacts.rs](../../../crates/worker/tests/brain_io/artifacts.rs) |
| 文件缺失、摘要不符与路径逃逸 | `missing_bad_digest_and_escaping_artifacts_fail_the_execution` | [artifacts.rs](../../../crates/worker/tests/brain_io/artifacts.rs) |
| 软链接文件拒绝 | `symlinked_declared_file_is_rejected_before_publishing_a_reference` | [artifacts.rs](../../../crates/worker/tests/brain_io/artifacts.rs) |
| 大索引清单、实际文件摘要与证据容量 | `large_file_index_uses_a_verified_manifest_and_keeps_decision_evidence_bounded` | [artifacts.rs](../../../crates/worker/tests/brain_io/artifacts.rs) |
| Worker 重启保留输入、结果、引用及执行身份 | `worker_restart_keeps_frozen_inputs_outputs_and_file_references_without_duplicate_execution` | [recovery.rs](../../../crates/worker/tests/brain_io/recovery.rs) |
| DAG 恢复拒绝输入漂移 | `dag_resume_rejects_input_drift_and_keeps_the_original_execution_identity` | [recovery.rs](../../../crates/worker/tests/brain_io/recovery.rs) |
| 原生动态恢复保留完整输入、成功检查点与资源副本 | `resume_freezes_inputs_skips_success_and_never_appends_twice` | [dynamic/recovery.rs](../../../crates/dag-runtime/tests/dynamic/recovery.rs) |
| 快照缺失时拒绝恢复，不重新生成输入 | `resume_rejects_missing_frozen_inputs_without_running_a_step` | [dynamic/recovery.rs](../../../crates/dag-runtime/tests/dynamic/recovery.rs) |
| 普通及共享计划拒绝跨 run、旧 activation，接受当前失败证据 | `ordinary_and_shared_plans_reject_foreign_and_old_inputs_but_allow_latest_failure` | [bindings.rs](../../../crates/brain/tests/milestone/bindings.rs) |
| Worker 拒绝外部运行的文件索引且保持原状态 | `worker_rejects_a_foreign_artifact_index_without_advancing_the_root` | [context.rs](../../../crates/worker/tests/brain_io/context.rs) |
| 托管 Agent、Team、TODO 的 HTTP 文件下载及普通执行门禁 | `managed_agent_team_and_todo_artifacts_stream_while_ordinary_executions_stay_rejected` | [managed.rs](../../../crates/control/tests/e2e/executions_artifacts/managed.rs) |
| 历史绑定和新 output/artifacts 选择 | `execution_sources_preserve_old_bindings_and_reject_unknown_sources` | [capability.rs](../../../crates/core/src/brain/capability.rs) |
| 历史文件引用和第 0 个动态实例 | `artifact_references_read_historical_files_and_preserve_zero_instance_index` | [run.rs](../../../crates/core/src/brain/run.rs) |
| 清单接口与业务步骤的名称冲突在定义校验时拒绝 | `managed_manifest_step_name_cannot_shadow_business_step_artifacts` | [artifacts.rs](../../../crates/dag/src/artifacts.rs) |

## 验证状态

本地实现和隔离验收完成；未执行全量测试，未发布线上版本。定向结果按不同用例去重记录，分组测试不写成全量通过。

| 实际范围（`cargo test -p <crate>`，均使用 `--locked`） | 通过数 |
| --- | ---: |
| `opencoder-core --lib brain::`：绑定与引用兼容 | 3 |
| `opencoder-dag --lib artifacts::` 及新增清单保留名用例 | 11 |
| `opencoder-brain --test milestone`：含返工、屏障、来源校验 | 18 |
| `opencoder-dag-runtime --lib exec::native::artifacts::` | 2 |
| `opencoder-dag-runtime --test dynamic`：完整输入、取消、失败收尾和恢复 | 11 |
| `opencoder-store --test brain_layered_v4 current_error_recovery` | 2 |
| `opencoder-control --lib`：capabilities 与 gateway 两组过滤 | 4 |
| `opencoder-control --test e2e`：artifacts、layered_api、registered operators 三组过滤 | 28 |
| `opencoder-control --test release_handoff layered_retry_keeps_original_intent_after_server_handoff` | 1 |
| `opencoder-worker --lib`：workspace 与失败证据两组过滤 | 4 |
| Worker 的 `brain_contracts`、`brain_dispatch_failures`、`brain_scheduler_v4`、`brain_closed_loop`、`brain_nested`、`brain_server_restart`、`brain_context_budget` | 23 |
| `opencoder-worker --test brain_io`：分组核验输入、文件、不同能力、不同节点、并发及重启 | 12 |

- Clippy：`cargo clippy -p opencoder-core -p opencoder-dag -p opencoder-dag-runtime -p opencoder-brain -p opencoder-store -p opencoder-control -p opencoder-worker --all-targets -- -D warnings` 通过；后续只对变化的 DAG、DAG runtime 和 Worker 重查，零警告。
- 新增代码文件不超过 400 行，修改文件不超过 800 行；格式、差异空白和相关文档链接检查通过。

已完成隔离环境实际进程验证：

- `scripts/acceptance/runc_scheduling/main.py`：两节点 20 次真实 runc 运行、动态实例、16 MiB 文件、失败/超时/取消及 Worker 重启；固定版本恢复通过，源工作区不变，清理后无剩余挂载，三个进程正常退出。
- `scripts/acceptance/storage_root/main.py`：最终产物的独立 Runtime 切换、回滚及原执行归属读取通过；163 次公共探测零失败，最大耗时或间隔 0.203 秒，源工作区内容、权限和归属保持不变。
- 进程产物为同一构建信息的五个冻结 debug 二进制，基线 `653df751` 且包含工作树改动；这些证据是本地隔离验收，不是正式发布包或线上发布回执。

原始日志、五个最终进程文件的摘要与构建信息、去重测试清单保存在 `/tmp/opencoder-execution-io-resume-20261013/`；runc 最终进程回执在 `/root/.cache/opencoder-e2e/20261013-execution-io-final/evidence/result.json`，切换回滚回执在本机同目录的 `opencoder-smooth-u3ygkmno/storage-result.json`。

## 相关约定

[规则 04](../../../rules/04-dag-execution-contract.md) · [规则 06](../../../rules/06-brain-scheduling-contract.md) · [运行协议](../../../docs/brain-orchestration.md) · [Brain 逻辑](../../../agents/brain/index.md) · [Worker 逻辑](../../../agents/worker/index.md)
