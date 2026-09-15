Commit: 8a50a393cbe615f5d6453ff4290da0bf03546881

# todos 模块

执行预编译 WorkflowSpec 的持久化 TODO 工作流运行时。

## 关键路径
- `src/types.rs` — `WorkflowSpec`/`TodoSpec`/`WorkflowState`/`ParentDecision`/`ContextMode`。
- `src/domain.rs` — `validate_spec`；TODO agent 必须为 primary。SPA 镜像 `crates/web/spa/src/todo/directory/specValidate.js` 逐条对齐（改校验规则需同步）。另有 TODO env 三件套：`env_vars_from_context`（校验+排序）、`env_vars_metadata`（盖章形状）、`env_passthrough_from_metadata`（运行时提取）。
- `tests/env_passthrough.rs` — 证明盖章的 `metadata.env_vars` 真正抵达子会话 bash 进程（`env_passthrough` → `ToolContext::extra_env`）。
- `src/parent.rs` — 父 workflow Primary Session 决策循环；调度读取精简状态与结果摘要，验收读取当前候选及工具门禁。
- `src/review/context.rs` — 纯函数构造子任务上下文：目标、约束、TODO、已验收依赖结果与证据、恢复信息和重跑原因；同一对象同时用于派发留痕与子会话输入。
- `src/review/rerun.rs` — 任意 TODO 重跑的前置守卫、下游闭包和状态变换；调用方先停止旧驱动再应用。
- `src/batch.rs` — 批量独立派发，持久化每项 assignment 的会话、尝试和上下文；候选事件携带实际门禁结果。
- `src/execution.rs` — 每 TODO 独立 act Primary Session 完成闭环；dispatch 后把 `workflow.metadata.env_vars` 并入 `session.env_passthrough`（BTreeMap 合并，TODO env 覆盖 resume 恢复的 harness env）。
- `src/transitions.rs` — 状态机守卫；Interrupted 豁免 max_attempts。
- `src/persistence.rs` — generation CAS 单事务提交投影与事件；事件附带提交后的 generation、world_epoch 和节点状态。
- store 表 `todo_workflows`/`todo_items`/`todo_events`；SSE 轮询在 `crates/web/src/todo_hub.rs`。
- `src/directory/` — JSON/Markdown 文件集与 WorkflowSpec 的无损编解码、文件路径诊断、环境绑定及目录 IO；`load_bound` 是 CLI 与模板派发的共同入口，旧 `context.json` 只读兼容。
- `directory::write_new` 校验后通过 staging + rename 写入新目录，拒绝覆盖；模板版本由 Web/Control 的目录 API 管理。CLI 入口 `crates/local/src/todos_cmd.rs`。
- 测试：`crates/todos/tests/`（runtime/recovery/interrupt/guards 等）。

## 边界
- 模板目录是定义来源；运行先冻结定义，Store 是运行状态与事件的唯一权威。`--debug` 文件目录不反向覆盖。
- 文件形状、缺失/多余文件、任务 ID、依赖与环境绑定由服务端统一校验，诊断携带路径和可用的行列；无效目录不得保存或运行。
- TODO env 是唯一的“环境”体系：节点侧 OpenCoder Env 配置集（`/api/envs`）已整体删除；worker `input.envs` 是另一体系（harness per-task 进程 env），勿混淆。
- 父 Session 不执行 TODO 工具；子 Session 不写工作流投影。
- 运行定义冻结。重跑递增 world_epoch，重置目标及下游，保留上游、独立分支结果和原会话历史；fork 创建新的独立会话，原会话只供回看。文件和外部操作结果由当前工作区保留。

## 相关
- [agents/local](../local/index.md) — CLI 入口。
- [agents/store](../store/index.md) — 持久化合同。
- [agents/worker](../worker/index.md) — 持久化重跑受理、停止与重新排队。
- [features/todos](../../features/todos/index.md) — 用户能力。
