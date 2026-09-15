Commit: 8a50a393cbe615f5d6453ff4290da0bf03546881

# dag 模块

零 IO 的 DAG 纯域 + 线协议，server 与节点两端唯一契约。

## 关键路径
- `src/spec.rs` — `DagSpec`/`validate`/`StepKind::{Agent,Wasm}`；`decode_spec` 通过类型反序列化拒绝未支持的步骤类型；无注册 Runner 专用解析或执行分支。
- `src/domain.rs` — `ready_steps`/`run_outcome`/`render_context` 运行态推进。
- `src/transitions.rs` — 状态机纯函数，终态冻结。
- `src/artifacts.rs` — `/workflow/<run_id>/<step>/` 工件契约；run id 拒绝 `_modules`；`session_file`/`session_value`/`parse_session_id` 定义 `session.json`（`{"session_id":"<ulid>"}`，步骤运行中的实时指针），`meta_value_with_session` 让 `meta.json` 携带可选 `session_id`（旧文件仍可解析）。
- `src/protocol.rs` — 线协议 DTO LOCKED，server 只存转不执行；`step_log` 事件承载步骤的增量日志。
- 持久化在 store 层：`dag_defs`/`dag_runs`/`dag_events`（schema v16），本 crate 不触 SQL。

## 边界
- 纯域零 IO；环/缺边由 server 拒绝，节点侧防御性复验。

## 相关
- [agents/dag-runtime](../dag-runtime/index.md) — 节点执行方。
- [agents/dag-wasm](../dag-wasm/index.md) — wasm 模块版本池：发布/NFS 导出/节点冻结分发。
