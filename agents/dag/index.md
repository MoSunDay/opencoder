# dag — DAG 工作流纯域层

`crates/dag`：零 IO 的 DAG workflow 纯域 + 线协议。被 server（校验/展示）与 dag-runtime（节点执行）共享，是两端的唯一契约来源。

## 结构

- `spec.rs` — `DagSpec`/`StepSpec` 声明与 `validate`（唯一入口校验：slug 合法性、重复边、环检测、缺依赖）；`StepKind::{Agent, Python}`（Python 携带 `sandbox: Option<SandboxMode>`，默认 InProcess）。默认值与 `serde` 反序列化宽容。
- `domain.rs` — `StepStates`/`StepOutputs` 运行态推进：`ready_steps`（依赖全 Done 且未在运行/终态）、`run_outcome`（cancelled > error > done 折叠）、`render_context`（上游 outputs 注入 step 上下文 JSON）。
- `transitions.rs` — 状态机纯函数（Running→Done|Error|Cancelled，终态冻结）。
- `artifacts.rs` — 节点本地工件目录契约：`/workflow/<run_id>/<step>/{output.json,output.txt,meta.json}`；`output_snapshot`（4KB 截断快照随 step_done 事件上行）、`meta_value`。
- `protocol.rs` — **线协议（LOCKED）**：`DagDefPayload`/`DagClaimedRun`/`DagEventIn`/`DagEventBatch`/`DagStatusReport`；事件种类 `run_started|step_started|step_done|run_finished`。

## 约定

- 依赖图用 slug（kebab-case）标识 step；环/缺边在 dispatch 前被 server 拒绝，节点侧防御性复验。
- 持久化在 store 层（`libsql_store/{dag,dag_events}.rs`，schema v16），本 crate 不触 SQL。
