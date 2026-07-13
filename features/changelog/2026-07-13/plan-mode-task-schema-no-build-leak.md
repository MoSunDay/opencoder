# Plan-mode `task` 工具 schema 不再泄漏 `build` 子代理

## 背景

plan 模式有三层防护：系统提示词文本（`base_prompt_plan()` 的 `.replace()`）、
工具名白名单（`agent.tools`）、运行时拦截（`runner.rs` 拒绝 plan+build）。
但**工具 schema 层泄漏**：`schema_for(&allowed)` 把 `TaskTool` 的
`description()/parameters()` 原样塞进发给 LLM 的工具 JSON，其中始终写着
`subagent_type "build" (full tools)`。于是 plan 模式下模型仍被告知 `build`
可用，尝试后才被运行时拦截 ——「先误导再报错」。

根因：`Tool` trait 的 `description(&self)`/`parameters(&self)` 只有 `&self`，
无法感知 mode；schema 拼装层此前也未传入 agent kind。

## 变更

### A — plan 模式专用 schema（`crates/session/src/tools/task.rs`）
- 新增 `pub fn description_plan() -> &'static str`：与 `TaskTool::description`
  相同，但删去 `build` 从句，只广告 `explore`。
- 新增 `pub fn parameters_plan() -> Value`：与 `TaskTool::parameters` 相同，
  但 `subagent_type` 描述只提 `explore`。

### B — `schema_for` 感知 plan 模式（`crates/session/src/tools/mod.rs`）
- `schema_for` 签名增加 `kind: AgentKind` 参数。
- 当 `kind == Plan && name == "task"` 时改用 `task::description_plan()`/
  `task::parameters_plan()`；其余工具/其余 kind 行为不变。
- 在 schema 拼装层特判 `task`，隔离性好，不触动 `Tool` trait 签名（不影响
  其余 7 个工具）。

### C — 唯一调用点更新（`crates/session/src/runner.rs`）
- `schema_for(&allowed)` → `schema_for(&allowed, session.agent.kind)`。
- 运行时拦截（runner.rs plan+build 拒绝）**保留**作为兜底，不依赖 schema
  正确性。

### D — 契约测试（`crates/session/src/tools/mod.rs` 内 `#[cfg(test)]`）
- `plan_mode_task_schema_omits_build`：plan 模式下 `task` 的 description、
  `subagent_type` 描述、以及整个 parameters 块均不含 `build`，且含 `explore`。
- `act_mode_task_schema_advertises_build`：act 模式下 description 与
  `subagent_type` 描述均含 `build`（防回归）。
- `non_task_tools_unaffected_by_kind`：非 `task` 工具（`read`）在 plan 模式
  schema 不受 `kind` 参数影响。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| plan 模式 task schema 不含 build | `tools::tests::plan_mode_task_schema_omits_build` | `session/src/tools/mod.rs` |
| act 模式 task schema 仍含 build | `tools::tests::act_mode_task_schema_advertises_build` | `session/src/tools/mod.rs` |
| 非 task 工具不受 kind 影响 | `tools::tests::non_task_tools_unaffected_by_kind` | `session/src/tools/mod.rs` |
| plan 模式运行时拦截 build（兜底） | `plan_mode_blocks_build_subagent` | `session/tests/plan_mode_subagent_guard.rs` |
| plan 模式允许 explore | `plan_mode_allows_explore_subagent` | `session/tests/plan_mode_subagent_guard.rs` |
| act 模式允许 build（正向） | `act_mode_allows_build_subagent` | `session/tests/plan_mode_subagent_guard.rs` |

## 回归

`cargo test --workspace` 全绿（core/llm/store/session/web/cli/tui 全 0 failed）。
