Commit: d146e517f8b31ba3f8e5a1e493e0450d0c50d624

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 索引
- `src/bootstrap.rs` — control.db + definitions.db 装配
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- `src/api/session.rs` — 会话执行面：`POST /api/sessions` 按 `kind` 映射（缺省
  operator；`agent` 走同一执行器，其余 400），缺省 id 按 `kind.prefix()` 生成
  （agent-{ulid}/operator-{ulid}），整个 body 即 execution input（`prompt`/
  `how_append`/`agent`/`node_id` 在顶层）；列表合并 Operator+Agent 两族索引
  （created_at DESC + id ASC，500 上限），dag/team/maintenance 不混入
- `src/api/executions/` — 派发去重、选点冻结、回执；列表端点在 JSON 层提升顶层 `name`（派发时快照，按 kind 取定义名/target，见 `paging.rs`），执行索引五字段协议不动；非 admin 角色门禁放行 Operator/Agent 两族（提交与命令同规则，dag/team 403）；Agent kind 的 `input["how_append"]` 在准入即校验 8 KiB 预算（对齐 `opencoder_dag::spec::MAX_HOW_APPEND_BYTES`）
- `src/api/catalog.rs` — 节点与定义目录
- `src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由；`clear_dialogs` 是节点中继：
  fleet.nodes 404 → indexes(Operator) 算 drop_ids → hub.call(Maintenance dialogs_clear) →
  delete_terminal_indexes；节点不识别该操作时原样透传节点错误（fail-closed，防索引行复活）
- `src/api/project.rs` — Project overview/runs 对节点 `404 execution not found`（journal 丢失/节点重建）按持久索引静默降级：overview 行不带 `detail_error`、runs 返回空页，与 `Ok(None)` 无索引同形；其余失败（节点离线 503 等）保持 `detail_error`/透传
- `src/api/stream.rs` — 分页→SSE 事件流
- `src/api/brain_runs/` — v2 计划注册、能力目录、版本快照、运行提交与持久化后派发。
- `src/scheduler.rs` — schedules.json cron 调度循环（仅控制面运行）：24h 追赶窗内只 fire 最新 due tick、更老 tick 折叠一条 `missed` 代表行；`overlap: skip` 看上一条 fired 行的执行索引；error 行 1h 重试窗内原地重试
- `src/api/schedules/` — `GET /api/schedules`（定义 + last_run/next_run）、`GET /api/schedules/:id/runs?limit=`；admin-only

## Brain 接缝

计划发布检查 Agent/DAG/Team/TODO/Operator 注册身份并固定能力定义、执行配置和资源；相同版本重试复用既有快照。动态生成完整计划后也经过相同发布入口。运行经 `/api/brain/runs` 提交，通用执行 API 拒绝绕过该契约直接创建 Brain。

旧决策树、Playbook 及 Project 的旧 Brain 执行模式返回迁移错误；历史查询保留。节点选点沿用 Fleet 规则，明确指定的子执行位置不会被根节点覆盖。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
