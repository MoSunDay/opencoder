Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# dag 模块

零 IO 的 DAG 纯域 + 线协议，server 与节点两端唯一契约。

## 关键路径
- `src/spec.rs` — `DagSpec`/`validate`/`StepKind::{Agent,Wasm,Runner}`；python 步骤已下线，`decode_spec` 专用报错不静默迁移。
- `src/domain.rs` — `ready_steps`/`run_outcome`/`render_context` 运行态推进。
- `src/transitions.rs` — 状态机纯函数，终态冻结。
- `src/artifacts.rs` — `/workflow/<run_id>/<step>/` 工件契约；run id 拒绝 `_modules`。
- `src/protocol.rs` — 线协议 DTO LOCKED，server 只存转不执行。
- 持久化在 store 层：`dag_defs`/`dag_runs`/`dag_events`（schema v16），本 crate 不触 SQL。

## 边界
- 纯域零 IO；环/缺边由 server 拒绝，节点侧防御性复验。

## 相关
- [agents/dag-runtime](../dag-runtime/index.md) — 节点执行方。
- [agents/dag-wasm](../dag-wasm/index.md) — wasm 模块版本池：发布/NFS 导出/节点冻结分发。
