Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# todos 模块

执行预编译 WorkflowSpec 的持久化 TODO 工作流运行时。

## 关键路径
- `src/types.rs` — `WorkflowSpec`/`TodoSpec`/`WorkflowState`/`ParentDecision`/`ContextMode`。
- `src/domain.rs` — `validate_spec`；TODO agent 必须为 primary。SPA 镜像 `crates/web/spa/src/todo/editor/specValidate.js` 逐条对齐（改校验规则需同步）。另有 TODO env 三件套：`env_vars_from_context`（校验+排序）、`env_vars_metadata`（盖章形状）、`env_passthrough_from_metadata`（运行时提取）。
- `tests/env_passthrough.rs` — 证明盖章的 `metadata.env_vars` 真正抵达子会话 bash 进程（`env_passthrough` → `ToolContext::extra_env`）。
- `src/parent.rs` — 父 workflow Primary Session 决策循环。
- `src/execution.rs` — 每 TODO 独立 act Primary Session 完成闭环；dispatch 后把 `workflow.metadata.env_vars` 并入 `session.env_passthrough`（BTreeMap 合并，TODO env 覆盖 resume 恢复的 harness env）。
- `src/transitions.rs` — 状态机守卫；Interrupted 豁免 max_attempts。
- `src/persistence.rs` — generation CAS 单事务提交投影与事件。
- store 表 `todo_workflows`/`todo_items`/`todo_events`；SSE 轮询在 `crates/web/src/todo_hub.rs`。
- spec 经 core `share_fs`（`context.json`）下发；CLI 入口 `crates/local/src/todos_cmd.rs`。
- 测试：`crates/todos/tests/`（runtime/recovery/interrupt/guards 等）。

## 边界
- Store 唯一权威；`--debug` 文件目录不反向覆盖。
- TODO env 是唯一的“环境”体系：节点侧 OpenCoder Env 配置集（`/api/envs`）已整体删除；worker `input.envs` 是另一体系（harness per-task 进程 env），勿混淆。
- 父 Session 不执行 TODO 工具；子 Session 不写工作流投影。

## 相关
- [agents/local](../local/index.md) — CLI 入口。
- [agents/store](../store/index.md) — 持久化合同。
- [features/todos](../../features/todos/index.md) — 用户能力。
