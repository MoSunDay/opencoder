Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# Dynamic DAG：实例 how 副本与 argv 批次

## 变更

- 新增 `dynamic` 模板节点，从派发 input 或已成功上游的结构化输出展开最多 1,000 个 Agent/Wasm 实例；空批次输出 `[]`，非法输入完整校验后拒绝执行。
- Agent 冻结原始资源，实例 how 副本依次追加公共 how_append 与实例文本，Host/runc 共用；恢复复用副本，DAG 成功后不再回写共享资源。
- Wasm 直接追加实例 argv；rootfs 运行器支持 OCI 缓存配置，模块参数中的空格、`--env`、`--dir` 保持原样。
- 共享每 run 四名额并轮询就绪节点；同组失败停止派发、取消并收齐同组实例，独立分支继续，成功输出按输入顺序聚合。
- 原子展开清单、实例回执、分页查询、实例 SSE 和产物 index 贯穿节点与控制台；重试清理旧状态，切换实例清理旧订阅，错误提示不被无关刷新掩盖。
- 为通过工作区结构门禁，将 Web drain 生命周期提取到独立模块，聚合执行配置；产物读取复用统一请求 DTO。

## 兼容

静态 Agent/Wasm 定义继续可读，静态 Host Agent 保留原有工作目录。旧 Worker 对动态类型明确拒绝。实例位于 `<run>/<step>/instances/<index>/`；未新增数据库表或环境变量。普通 Agent 会话的 how 资源追加独立于 DAG 的副本机制。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| Agent how 隔离及有序输出 | `agent_inputs_enter_distinct_how_copies_and_outputs_keep_input_order` | [runtime dynamic](../../../crates/dag-runtime/tests/dynamic/main.rs) |
| 上游来源、Wasm argv | `upstream_output_expands_wasm_argv_without_splitting_spaces` | [runtime dynamic](../../../crates/dag-runtime/tests/dynamic/main.rs) |
| 四名额、公平调度、失败分支隔离 | `four_slots_round_robin_and_failed_group_do_not_cancel_independent_branch` | [runtime dynamic](../../../crates/dag-runtime/tests/dynamic/main.rs) |
| 空批次、类型错误、超限 | `empty_batch_succeeds_and_invalid_input_never_partially_expands` | [runtime dynamic](../../../crates/dag-runtime/tests/dynamic/main.rs) |
| 冻结输入、跳过成功、不重复追加、源版本不变 | `resume_freezes_inputs_skips_success_and_never_appends_twice` | [recovery](../../../crates/dag-runtime/tests/dynamic/recovery.rs) |
| 实例超时、用户取消 | `timeout_is_per_instance_and_user_cancel_is_run_cancellation` | [recovery](../../../crates/dag-runtime/tests/dynamic/recovery.rs) |
| 持久化失败时取消并收齐 | `scheduling_persistence_failure_cancels_and_drains_live_instances` | [persistence](../../../crates/dag-runtime/tests/dynamic/persistence.rs) |
| 千级分页、身份校验及恢复日志过滤 | `thousand_instances_page_cap_detail_and_identity_validation`、`instance_logs_filter_index_replay_cursor_and_agent_session` | [worker query](../../../crates/worker/src/operations/query/tests/dag_step_events/instances.rs) |
| HTTP 页大小、SSE 游标和错误 | `instance_routes_preserve_identity_and_bound_pages`、`instance_sse_reconnects_at_cursor_and_returns_worker_errors` | [control e2e](../../../crates/control/tests/e2e/dag_instances.rs) |
| 真实 Wasm/Agent/runc、实例产物和日志 | `dynamic_wasm_instances_have_http_pages_isolated_argv_artifacts_and_replay`、`runc_dynamic_agent_and_wasm_read_isolated_copies_and_argv` | [process e2e](../../../tests/dag_e2e/dynamic.rs) |
| CLI 缓存选项及模块参数边界 | `parses_the_bundle_argv_shape` | [wasmtime-cli](../../../crates/dag-runtime/examples/wasmtime-cli.rs) |
| 分页选择、切换清理、终态回放、错误保留及同 run 恢复 | `instances.dom.test.jsx` 的五项行为测试 | [SPA](../../../crates/web/spa/src/dag/dynamic/instances.dom.test.jsx) |
| Server→Worker→浏览器、两种来源、历史与重连 | `dag_dynamic.js` | [Chromium 验收](../../../scripts/acceptance/dag_dynamic.js) |

## 验证记录

- DAG、运行时、控制面相关 Rust 套件通过；实例运行时 7 项、Worker 日志/查询 11 项、运行进度 2 项通过。
- 真实进程验收 2 项通过，包含可用 runc 下的实际容器执行；浏览器两种来源、输入提交、实例切换、历史回放和连接中断恢复通过。
- 最终独立快照 `522cd534` 的 Server→Worker→Chromium 验收通过（`/tmp/dynamic-browser-complete.log`）；实例终态、分页选择及日志截图与回执位于 `/tmp/opencoder-todo-workbench-lsBkNO/`。
- SPA 全量 118 个文件、871 项测试通过（`/tmp/dynamic-spa-resume-complete.log`）；最终 SPA 构建通过（`/tmp/dynamic-spa-build-complete.log`）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零告警通过。
- 工作区全量测试与最终构建：验证中，完成后补充实际结果。

## 相关说明

[使用与 API](../../../docs/dag-dynamic.md) · [纯域](../../../agents/dag/index.md) · [运行时](../../../agents/dag-runtime/index.md) · [平台行为](../../agent-platform/index.md)

全量回归中另行处理的 [Brain 回执与报告通知](brain-wake-receipts.md)、[Host 容量轮询](host-capacity-read-only.md)、[TODO 兼容接口测试准备](todo-interrupt-fixture.md) 均保留了对应测试和原因记录。
