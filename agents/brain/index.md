Commit: (working-tree, 基于 b465f440)

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

- `src/playbook/` — 剧本双轨纯域：`spec.rs`（`PlaybookSpec` 聚合校验/`validate_draft`/`render_prompt`）、`topology.rs`（Kahn 拓扑/`collapse_blocked`）、`trigger.rs`（message 相似度触发/`scan`）。
- `src/planning.rs` — `plan_playbook`：LLM 铸动态剧本、候选背靠背校验、digest 缓存复用（空库退单步 {Agent,"act"}）。
- `src/runtime.rs` — playbook CRUD（`PLAYBOOK_ID_PREFIX="playbook"`）；store 表 v25 `brain_playbooks`。
## 边界
- 能力绑定与派发在 control（request_id 幂等）；执行明细落 worker 节点。
- Agent 绑定聚合视图 `GET /api/brain/agents`（control `api/brain.rs`）：按 `capability_target` kind=Agent 分组出 `{agent, capabilities:[{id,summary}]}`，team resolve 用它固化成员能力快照。

- 剧本固定/动态两来源同走 `PlaybookSpec`；本地执行在 project（`executor/playbook_drive.rs`），平台展开派发在 control（`api/brain_playbook_dispatch.rs`）。
## 相关
- [agents/control](../control/index.md) — 绑定与派发。
- [agents/worker](../worker/index.md) — 执行明细归属。
- [agents/store](../store/index.md) — 向量三表与 plans。
- [agents/llm](../llm/index.md) — embed 接口。
- [agents/web](../web/index.md) — HTTP 路由。
