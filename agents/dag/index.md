Commit: 2491657d33c384dddcabf4d12ab4cd8822ccaf81


`crates/dag`：零 IO 的 DAG workflow 纯域 + 线协议。被 server（校验/展示）与 dag-runtime（节点执行）共享，是两端的唯一契约来源。

## 结构

- `spec.rs` — `DagSpec`/`StepSpec` 声明与 `validate`（唯一入口校验：slug 合法性、重复边、环检测、缺依赖）；`StepKind::{Agent, Wasm, Runner}`：Agent 携带 `prompt`/`agent`（默认 act）/`model`/`how_append`（≤`MAX_HOW_APPEND_BYTES`），Wasm 携带 `command`（`"<module.wasm> [args...]"` 空白切分）与 `sandbox: Option<SandboxMode>`（默认 InProcess）。默认值与 `serde` 反序列化宽容。`decode_spec`/`decode_spec_str`（字符串入口）是旧定义的迁移哨兵：python 步骤专用报错（"该定义使用已下线的 python 步骤…"），不静默迁移；worker create/workloads、control catalog、project dag_drive、web defs 读写全部经此统一报错（web 列表对坏行降级为带 error 的行，不再整页 500）。
- `domain.rs` — `StepStates`/`StepOutputs` 运行态推进：`ready_steps`（依赖全 Done 且未在运行/终态）、`run_outcome`（cancelled > error > done 折叠）、`render_context`（上游 outputs 注入 step 上下文 JSON）。
- `transitions.rs` — 状态机纯函数（Running→Done|Error|Cancelled，终态冻结）。
- `artifacts.rs` — 节点本地工件目录契约：`/workflow/<run_id>/<step>/{output.json,output.txt,meta.json}`；`output_snapshot`（4KB 截断快照随 step_done 事件上行）、`meta_value`。
- `protocol.rs` — **线协议（LOCKED）**：`DagDefPayload`/`DagClaimedRun`/`DagEventIn`/`DagEventBatch`/`DagStatusReport`；事件种类 `run_started|step_started|step_done|run_finished`。

## 约定

- 依赖图用 slug（kebab-case）标识 step；环/缺边在 dispatch 前被 server 拒绝，节点侧防御性复验。
- 持久化在 store 层（`libsql_store/{dag,dag_events}.rs`，schema v16），本 crate 不触 SQL。

Runner 步骤以 `runner` 指向注册执行入口，以 `agent` 指向 Codex 引用卡；域层仅校验结构与名称，不启动进程。它沿用 Step 的依赖、超时、上下文和产物契约；节点执行见 [dag-runtime](../dag-runtime/index.md)。
