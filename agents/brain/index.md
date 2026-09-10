Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# brain 模块

能力库 + 向量检索 + 决策树路由规划。

## 关键路径
- `src/domain.rs` — `validate`/`compose_embed_text`/LE f32 编解码。
- `src/runtime.rs` — `Runtime`：upsert/update 单事务组合写、余弦 `search`。
- `src/plan.rs` — `DecisionTree`/`PlanNode` 纯域校验与 dispatch。
- `src/planning.rs` — `plan_decision_tree`/`dispatch_or_plan`（situation digest 缓存）。
- `src/error.rs` — typed marker：`EmbeddingFailed`/`BrainNotFound`。
- store 表 v15 `brain_*` 三张、v18 `brain_plans`；embed 走 `ChatStream::embed`。
- web 路由 `crates/web/src/api_brain.rs`；SPA 面板 `crates/web/spa/src/brainPanel.jsx`。
- 测试：`crates/brain/tests/{runtime,planning}.rs`、web 层 `crates/web/tests/web_brain*.rs`。

## 边界
- 能力绑定与派发在 control（request_id 幂等）；执行明细落 worker 节点。

## 相关
- [agents/control](../control/index.md) — 绑定与派发。
- [agents/worker](../worker/index.md) — 执行明细归属。
- [agents/store](../store/index.md) — 向量三表与 plans。
- [agents/llm](../llm/index.md) — embed 接口。
- [agents/web](../web/index.md) — HTTP 路由。
