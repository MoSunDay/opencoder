# 平滑发布：新任务切新版，已有任务保持 Runtime 归属

## 变更

- 拆分稳定 Agent Host 与版本 Runtime；持久化执行归属、全机并发容量和 FIFO。发布、回滚、Server/Host 退出不发送执行中断信号。
- Server 使用共享请求回执和冻结派发记录恢复未确认请求；Brain、Playbook 与 Project 路由使用跨进程锁。相同请求重放，同一 ID 的不同输入拒绝。
- Nginx 固定入口、完整索引预热、Host 递增编号交接、SSE 游标续接和独立只读 NFS。Runtime 满足执行、队列、工具及写入全部结束后才休眠，历史查询可唤醒。
- 发布命令持久化阶段、支持同 ID 续跑、兼容回滚、在线备份与首次迁移；管理页面展示旧版剩余任务及回收失败。
- 纯 WASM DAG 仅固定自身模块，不再复制无关 Agent 资源池；只读挂载检查保留，包含 Agent 步骤或 Agent 资源指纹的 DAG 继续冻结完整资源。
- 修复真实容器演练发现的 wasmtime 缓存路径问题：缓存使用 OCI 私有 `/tmp`，根目录继续只读。历史归属复核走只读查询，避免每次索引同步都争用写锁。
- 请求跟踪保留 HTTP 响应长度和尾部帧；各 Runtime 固定全局技能和已配置的 OCI 镜像，避免新版启动改写旧任务资源。
- Project 首次派发按 run_id 持久化回执；明确拒绝后的新规划仍保留原归属，旧请求重试返回原拒绝，未确认派发禁止替换。
- 每次回滚或重新激活使用新的持久探针编号，确认当前入口真正执行新任务；同一次发布中断续跑保留编号，避免重复探针。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 持久请求和冻结 assignment | `dispatch_survives_reopen_and_preserves_original_assignment` | `crates/store/tests/handoff.rs` |
| 跨进程锁释放 | `process_lock_serializes_independent_connections_and_releases_on_drop` | 同上 |
| 三版归属、容量、FIFO、回滚 | `ownership_and_fifo_capacity_span_three_releases_and_rollback` | 同上 |
| 旧 Host 报告隔离 | `stale_server_reports_cannot_overwrite_new_host_state` | 同上 |
| 历史路由不等待写事务 | `historical_owner_reads_do_not_wait_for_another_process_writer` | 同上 |
| 实际 Worker 保持模型调用及容量 | `three_runtime_versions_keep_live_model_calls_and_global_fifo` | `crates/agent/src/host/tests.rs` |
| Server 崩溃派发恢复 | `prepared_dispatch_recovers_on_another_server_and_retries_execute_once` | `crates/control/tests/release_handoff.rs` |
| Brain 能力快照重放 | `brain_run_retry_keeps_original_capabilities_after_server_handoff` | 同上 |
| SSE 退役与 admission 分离 | `retirement_interrupts_a_slow_sse_poll_and_preserves_cursor_and_admission` | 同上 |
| Project 拒绝后继续规划与历史回执 | `a_rejected_first_execute_does_not_prevent_later_planning`、`a_rejected_resource_preflight_keeps_affinity_and_allows_a_new_plan`、`only_a_definitively_rejected_project_attempt_can_be_replaced_and_receipts_survive_reopen` | `crates/worker/tests/project_replay.rs`、`crates/store/tests/project_dispatch.rs` |
| Project 未确认命令路由 | `keyed_project_routing_rejects_changed_intent_after_unconfirmed_command` | `crates/control/tests/e2e/project_api.rs` |
| CLI Host/Runtime 模式 | `independent_host_and_runtime_modes_parse_without_legacy_run` | `crates/agent/src/main.rs` |
| 纯 WASM 的资源依赖范围 | `wasm_only_dags_do_not_depend_on_unrelated_agent_pools` | `crates/worker/tests/resource_snapshot.rs` |
| OCI 缓存和只读根目录 | `container_config_shape`、`write_bundle_writes_config_and_private_rootfs` | `crates/dag-runtime/src/sandbox/oci.rs` |
| 全局技能按 Runtime 固定 | `two_releases_keep_skill_bytes_when_shared_source_changes`、`runtime_skill_discovery_stays_pinned_after_shared_skills_change` | `crates/core/src/skill/runtime.rs`、`crates/core/tests/runtime_skills.rs` |
| 跟踪响应保留传输契约 | `tracking_preserves_response_length_trailers_and_body_lifetime` | `crates/control/src/release/mod.rs` |
| 正常通道退役与异常关闭区分 | `connection_reads_calls_while_indexes_wait_and_cancels_collection_on_close`、`normal_retirement_and_error_close_codes_remain_distinct` | `crates/node/src/fleet/client/tests.rs` |
| 发布失败、回滚、兼容、systemd | `DeploymentTests` | `scripts/platform/rolling_tests/test_deployment.py` |
| 回滚后重新执行探针、续跑保持编号 | `test_retry_after_rollback_executes_new_probes_and_keeps_resume_identity` | 同上 |
| 首次迁移中断续跑 | `test_resume_after_current_pointer_was_written_finishes_ingress` | `scripts/platform/rolling_tests/test_migration.py` |
| 响应丢失后的探针回执恢复 | `ProbeTests` | `scripts/platform/rolling_tests/test_probes.py` |
| 24 个中断点、备份完整性 | `RecoveryTests` | `scripts/platform/rolling_tests/test_recovery.py` |
| 发布状态和 SSE 客户端 | DOM/游标测试 | `crates/web/spa/src/fleet/releases.dom.test.jsx`、`crates/web/spa/src/sse.release.test.js` |
| 真实进程、TODO 链、WASI、Shell、NFS、回滚 | `exercise` | `scripts/acceptance/smooth_release/main.py` |
| 真实模型依赖脚本与持久调度指标 | `AcceptanceTests` | `scripts/acceptance/smooth_release/tests/test_live.py` |

## 当前验证状态

实现及隔离验收已通过；首次生产迁移和真实模型切换后的 15 分钟观察尚未完成。

- Rust 完整回归：5,228 项通过、0 失败，6 项既有手工用例忽略。日志 `/tmp/opencoder-smooth-final-tests2.log`；Project 资源拒绝后的继续规划、原拒绝回执重放及心跳恢复均通过。
- 全目标 Clippy 零警告，workspace 构建通过；日志 `/tmp/opencoder-smooth-final-clippy5.log`、`/tmp/opencoder-smooth-final-build2.log`。
- 前端全量 96 文件、683 项通过，SPA 构建与漂移检查通过；日志 `/tmp/opencoder-smooth-combined-spa4.log`。
- 发布工具 14 项通过，包含 24 个故障子场景；旧安装/备份工具 19 项、真实模型验收夹具 3 项通过。日志 `/tmp/opencoder-smooth-final-rolling-tests.log`、`/tmp/opencoder-smooth-platform-current.log`、`/tmp/opencoder-smooth-final-acceptance-tests.log`。
- 最新隔离演练使用生产所在 ext4 磁盘，完成三版共存、两次 Host 交接、带任务回滚、Server SIGKILL 恢复、休眠/历史唤醒、全局容量/FIFO、技能隔离、只读 NFS；Shell、WASI 与真实 OCI 容器原进程保持。56 次持续提交零失败，最长接收 215 毫秒，最大接收间隔 315 毫秒、调度间隔 409 毫秒，SSE 149 毫秒且持久事件逐条一致。证据 `/var/tmp/opencoder-smooth-g42kp5bx/result.json`、`scheduling.json`；模型为本地夹具。
- 使用生产只读资源池的独立 Runtime 复测：纯 WASM 受理由修复前 8.75–22.23 秒降至 9–10 毫秒，执行完成约 228 毫秒；证据 `/var/tmp/opencoder-resource-preflight-8t2uvsur`、日志 `/tmp/opencoder-resource-preflight-fixed.log`。
- 联合代码与 `320dbbf3` 发布候选一致，保留 TODO 目录交付及已上线 DAG 修复；差异仅为本文和接口说明补充。该候选的优化发布包另有磁盘演练证据 `/var/tmp/opencoder-smooth-h7quack9/result.json`，详见 [TODO 目录交付](todo-directory-editor.md#联合发布验证)。
- Nginx 1.30.4 已安装，配置检查通过且未启动；原生产 Server/Agent 继续服务。首次迁移、真实模型样本及最终观察结果将在实际执行后补录。
