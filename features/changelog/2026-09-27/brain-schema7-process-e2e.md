Commit: fcc48dcbe1d5c207b88dc9ac4f578cd36c8946e9

# Brain 进程级 E2E 对齐 schema 7

`brain_layered_e2e` 的进程级夹具使用旧 schema 6，已与 Server 的 schema 7 准入契约脱节。夹具现在使用显式里程碑层、单个节点能力与层间 transition；模型 stub 按当前 `layer_id` 派发和评估。原有的准入、只读投影、层屏障、子任务隔离与完成事件检查保留。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 原始 Brain 入口拒绝绕过 | `raw_brain_submissions_stay_rejected_for_the_layered_canvas` | `tests/brain_layered_e2e/admission.rs` |
| 旧版本拒绝迁移 | `unknown_schema_versions_never_fall_back_to_a_writer` | `tests/brain_layered_e2e/admission.rs` |
| 非法计划在调度前拒绝 | `invalid_canvases_are_rejected_before_any_dispatch` | `tests/brain_layered_e2e/admission.rs` |
| 分层视图与 CLI 投影 | `layered_view_and_rounds_read_a_real_projection` | `tests/brain_layered_e2e/surface.rs` |
| 层屏障与完整执行 | `layered_canvas_holds_the_barrier_then_completes_through_the_closing_activation` | `tests/brain_layered_e2e/canvas.rs` |
| 旧 schema 4 可读但拒绝新执行 | `historical_dag_can_be_drawn_without_enabling_legacy_execution` | `crates/brain/tests/layered.rs` |
| 嵌套计划能力引用与循环门禁 | `nested_plans_validate_every_reference`、`cyclic_and_overdeep_plans_fail_before_dispatch` | `crates/control/src/api/brain_runs/plan_capabilities.rs` |
| v7 节点能力协商 | `layered_runs_require_the_v7_advertisement` | `crates/control/src/api/executions/capabilities.rs` |
| 旧节点拒绝接收分层子任务 | `old_nodes_are_excluded_from_layered_children_before_assignment` | `crates/control/tests/e2e/dag_instances.rs` |

- 专项回归：`cargo test --test brain_layered_e2e` → 5 passed / 0 failed。
- Brain 历史计划专项回归：`cargo test -p opencoder-brain --test layered` → 2 passed / 0 failed。
- 控制层能力计划专项回归：`cargo test -p opencoder-control --lib plan_capabilities` → 2 passed / 0 failed。
- 旧节点分层子任务进程回归：`cargo test -p opencoder-control --test e2e old_nodes_are_excluded_from_layered_children_before_assignment` → 1 passed / 0 failed。
