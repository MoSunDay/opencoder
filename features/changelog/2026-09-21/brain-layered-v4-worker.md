Commit: 7177f7987d3c3cdc7ed17864a4ffcf347bf4d182

# 大脑分层能力画布（v4）的节点侧调度与持久化

新增 `schema_version: 4` 的分层能力画布运行：计划先按依赖切层，节点只在所属层被决策时创建一次尝试，层屏障要求整层终结后才放行下一层。运行版本不可修改；未知 `schema_version` 显式报错，不猜测降级。v3 行为不变：外层接缝按 `schema_version` 分支（`==3` 走 v3、`==4` 走 v4），v3 的路由、回执、输出适配与 projection 语义逐字节保留。

域规则集中在 `crates/brain/src/layered/`（纯函数）：`layers`/`layer_of`/`ancestors`/`layer_context` 负责层级与上游绑定，`validate_plan`/`validate_request` 负责准入，`decide`/`activate`/`terminal`/`admit`/`command` 负责层决策与层屏障，`operation_id`/`execution_id` 生成身份，`PROMPT` 提供层决策提示词。上限：节点 ≤ 256、单层宽度 ≤ 32、嵌套深度 ≤ 3、`max_rounds` 1..=32（缺省 32）、节点尝试 1..=5（缺省 2）。层划分从不落库，一律重算，避免投影与计划漂移。

线协议 DTO 落在 `crates/core/src/brain/layered/`（`LayeredRequest`/`LayeredPlan`/`LayeredRun`/`LayeredOperation`/`LayeredContext`/`LayeredDispatchIntent`/`LayeredTerminalEvent` 等）。`LayeredRun.layer` 是已完成层数，正在决策的层恒为 `run.layer + 1`；`run.layer == total_layers` 时用空 `nodes` 的收口上下文完成运行并冻结 `summary`。

持久化新增 `brain_layered_runs`/`brain_layered_operations`/`brain_layered_events` 三张表（`crates/store/src/libsql_store/brain_layered/`）与 `brain_layered`/`commit_brain_layered`/`brain_layered_events` 三个 trait 方法，投影写入按 generation 栅栏且原子；Store 的 `SCHEMA_VERSION` 未由本轮推动（合并线上为 28，来自同批合并的发布提交自身）。

节点侧新增 `crates/worker/src/brain/v4/`（`mod`/`state`/`parent`/`output`/`outbox`/`api`/`run`，与 v3 同构）。根节点持运行投影与 operations，动作面为 `layered_wake`/`layered_dispatch`/`layered_cancel`/`layered_terminal`，ack 与副作用为 `layered_wake_ack`/`layered_authorize`/`layered_receipt`/`layered_dispatch_ack`/`layered_cancel_ack`，root↔child RPC 为 `layered_context`/`layered_block`/`layered_summary`（输出走 `layered_output`）；v4 终态回执注解为 `brain_layered_ack`，durable 状态注解为 `layered_intent`，决策注解为 `layered_decision`。子执行输入约定：根与嵌套输入带 `layered_request`，叶子子输入带 `brain_layered` 对象与 `parent{run_id,operation_id,node_id,layer}` 绑定。

重试身份固定为 `operation_id = "{run}#l{layer}#{node}#a{attempt}"`，`execution_id` 同源，因此重试是新增 operation 而非改写旧记录。`layered_authorize` 只栅栏本层（`intent.generation <= snapshot.run.generation`），重试提升 generation 后仍可授权，迟到回执按尝试号忽略。CLI `brain ontology activate` 同步支持 v4 与空节点收口（`crates/ctl/src/cmd/brain/ontology.rs`）。

控制面读写与 SPA 画布同批落地（`crates/control/src/api/brain_runs/v4/`、`crates/web/spa/src/brain/workbench/layered/`），其回执另记。探针在保留 `dag_dynamic_v1` 与 `brain_scheduler_v3` 的同时新增 `brain_scheduler_v4`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 每 generation 只发一次 wake，ack 后关闭，暂停/恢复换新 generation | `root_emits_one_layered_wake_until_control_acknowledges_it` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 安装层上下文后节点自行重新入队做一次层决策；派发帧与 durable intent 一致、伪造操作 409、ack 后停止重放 | `layered_context_requeues_the_idle_root_for_its_layer_decision` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 失败尝试按 `retry.max_attempts` 重试（不重复决策、不失败运行），层屏障放行后下一层绑定上游 execution | `failed_attempt_retries_then_the_barrier_opens_the_next_layer` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 空节点收口上下文完成根运行、summary 绑定进结果、终态不再派发 | `closing_context_completes_the_root_and_binds_the_summary` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 层拓扑（kahn 分层、非 JSON 顺序）、未知字段/外来 todo、能力恰好一个、宽度与深度上限、未知 schema 与失联父运行 | `kahn_levels_follow_edges_not_json_order`、`plan_rejects_unknown_fields_and_foreign_todo`、`node_requires_exactly_one_capability_and_unknown_edges_fail`、`dispatch_covers_the_layer_and_fixes_bindings`、`request_rejects_unknown_schema_and_lost_parent` | `crates/brain/tests/layered/validate.rs` |
| 层内并发与层屏障、重试与终态失败、暂停/取消停止派发、非祖先绑定拒绝、上一尝试的迟到终态被忽略、完成后 summary 冻结 | `parallel_nodes_in_layer_then_next_layer`、`retry_schedules_next_attempt_then_fails_run`、`cancel_and_pause_commands_stop_dispatch`、`binding_to_non_ancestor_rejected`、`late_terminal_from_previous_attempt_ignored`、`completion_requires_every_layer_and_freezes_the_summary` | `crates/brain/tests/layered/barriers.rs` |
| 投影按 generation 栅栏且原子、重试落成独立 operation、终态不可变且重开仍在、未知/伪造运行拒绝、Store `SCHEMA_VERSION` 未被 v4 推动 | `layered_projection_is_atomic_and_generation_fenced`、`retry_attempts_are_stored_as_separate_operations`、`terminal_attempt_is_immutable_and_survives_reopen`、`unknown_run_reads_as_none_and_forged_operations_rejected`、`store_schema_version_is_untouched` | `crates/store/tests/brain_layered_v4.rs` |
| v3 回归（唤醒、上下文重入队、失败折叠、端到端往返） | `root_emits_one_scheduler_wake_until_control_acknowledges_it`、`scheduler_context_requeues_idle_root_for_node_decision`、`failure::child_failure_cancels_inflight_sibling_without_another_model_decision`、`control_round_trip_dispatches_child_and_waits_for_terminal_barrier` | `crates/worker/tests/brain_scheduler_v3.rs` |
| CLI 内层激活按 v4 上下文发层契约并写回决策；空 `nodes` 走收口指令 | `layered_context_sends_the_layer_contract_and_writes_the_decision`、`closing_layered_context_uses_the_closing_instruction` | `crates/ctl/src/cmd/brain/ontology/tests.rs` |
| v2 计划兼容回归 | `saved_plan_runs_resolve_scope_inputs_and_persist_round_evidence`、`five_types_dispatch_in_two_rounds_and_retain_original_execution_metadata` 等 4 项 | `crates/worker/tests/brain_scheduler_plans.rs` |

- `cargo test -p opencoder-worker`：217 passed / 0 failed（40 个测试目标）。
- `cargo test -p opencoder-brain -p opencoder-store`：356 passed / 0 failed（70 个测试目标）。
- `cargo test -p opencoder-cli`：97 passed / 0 failed（13 个测试目标）。
- 格式：`rustfmt --check` 对 worker/brain/store/ctl/core 的本次改动与新增文件无差异（控制面与 Web 的 v4 文件属另一回执，未纳入本次核对）。
- `cargo check -p opencoder-worker -p opencoder-cli` 通过；`cargo clippy -p opencoder-worker -p opencoder-brain -p opencoder-store -p opencoder-cli --all-targets -- -D warnings` 零警告（含控制面/Web 传递依赖）。
- 全量 `cargo test --workspace` 与发布回执按迭代收口另行记录，本条不据此宣告上线。
- 全轮收口回执：[brain-layered-v4.md](brain-layered-v4.md) — 合并全部测试清单，并附全量回归、clippy、构建、行数与 SPA 的实跑数字。
