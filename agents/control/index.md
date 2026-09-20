Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 索引
- `src/bootstrap.rs` — control.db + definitions.db 装配
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）；普通查询/控制调用保持
  15 秒超时，`Create` 派发单独使用 60 秒受理窗口，以覆盖节点首次在 NFS 上复制
  Agent 资源快照的慢路径，避免 operator/agent 提交被误报为 504
- `src/api/session.rs` — 会话执行面：`POST /api/sessions` 按 `kind` 映射（缺省
  operator；`agent` 走同一执行器，其余 400），缺省 id 按 `kind.prefix()` 生成
  （agent-{ulid}/operator-{ulid}），整个 body 即 execution input（`prompt`/
  `how_append`/`agent`/`node_id` 在顶层）；`GET /api/sessions` 按
  `?kind=operator|agent` 严格分 lane（缺省 operator），每行返回 `kind`、`node_id`
  与 `execution_ref`，Agent 明细仍由 Operator-capable 节点索引并按显式引用打开；
  dag/team/maintenance 不混入
- `src/api/executions/` — 派发去重、选点冻结、回执；列表端点在 JSON 层提升顶层 `name`（派发时快照，按 kind 取定义名/target，见 `paging.rs`），执行索引五字段协议不动；非 admin 角色门禁放行 Operator/Agent 两族（提交与命令同规则，dag/team 403）；Agent kind 的 `input["how_append"]` 在准入即校验 8 KiB 预算（对齐 `opencoder_dag::spec::MAX_HOW_APPEND_BYTES`）
- `src/api/settings/` — Harness（codex）配置与节点调度配置：`GET/PUT /api/nodes/:id/scheduling`（admin-only）经 hub Maintenance RPC（`scheduling` 读 / `configure_scheduling` 写）转发节点；`NodeScheduling`（`crates/core/src/fleet/queue.rs`）`normalized()` 空白归一 null、`validate()` 要求 workdir 绝对路径，控制面预校验失败直接 400
- `src/api/catalog.rs` — 节点与定义目录
- `src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由；dispatch（DAG/TODO）
  的 execution input 取 body 顶层 `input`（null/缺失退化为 `{}`，worker 侧据此
  注入「执行要求」后缀并落 `input.json`）；`clear_dialogs` 是节点中继：
  fleet.nodes 404 → indexes(所选 kind) 算 drop_ids → hub.call(Maintenance dialogs_clear) →
  只按所选 kind 删除终态控制面索引；节点不识别该操作时原样透传节点错误（fail-closed，防索引行复活）
- `src/api/project.rs` — Project overview/runs 对节点 `404 execution not found`（journal 丢失/节点重建）按持久索引静默降级：overview 行不带 `detail_error`、runs 返回空页，与 `Ok(None)` 无索引同形；其余失败（节点离线 503 等）保持 `detail_error`/透传
- `src/api/compat/dag_instances.rs` — 动态实例列表和详情中继；列表默认 100、上限 200。`src/api/stream.rs` 中的实例 SSE 与 `src/api/streaming/artifact.rs` 的可选 index 贯穿 Server→Worker，详见 [动态步骤 API](../../docs/dag-dynamic.md)。
- `src/api/stream.rs` — 分页→SSE 事件流
- `src/api/brain_runs/` — v2 计划注册与兼容查询，以及 v3 能力目录、轮次调度、最小快照、事件页和控制命令。Control 只归一化能力目录、创建真实子执行并处理中继回执；根节点持有 v3 调度投影、generation 和模型激活，调度器只在终态事件确认后再次唤醒。
- `src/scheduler.rs` — cron 调度循环（仅控制面运行）：定义读 libsql `schedules` 表（v27 起事实源），24h 追赶窗内只 fire 最新 due tick、更老 tick 折叠一条 `missed` 代表行；`overlap: skip` 看上一条 fired 行的执行索引；error 行 1h 重试窗内原地重试；`scan_interval_secs` 仍从 schedules.json 热读（运维旋钮）
- `src/api/schedules/` — 定义 admin CRUD + 台账查询：`params` 按 kind 消费
  （agent/team/todos 读 `prompt`、dag 读 `args`——触发时由 worker 追加到
  每个 Wasm 步命令行、brain 读 `objective`/`inputs`/`mode`/`plan`；字符串值
  可带 `{{now…}}` 时间模板，fire 时刻渲染成执行 input），`GET/POST /api/schedules`、`PUT/PATCH/DELETE /api/schedules/:id`（PATCH 仅 `{"enabled": bool}`，重名 409 / 非法 body 400 / 未知 id 404）、`POST /api/schedules/:id/run` 手动立即触发（绕过 enabled/overlap）、`GET /api/schedules/:id/runs?limit=`；admin-only
- `src/seed_schedules.rs` — 一次性导入遗留 `schedules.json` 定义进 `schedules` 表：仅表空时执行（skip-don't-merge，删除不会在重启时复活），非法条目 warn 跳过不阻断启动；`ScheduleJob::validate` 是唯一校验门

## 内置 seed

- `src/seed_dags.rs` — serve_release 启动时把三个 review 门禁 def（review-full-acceptance / review-harness-quick / review-code-quick）seed 进 FleetStore `fleet_definitions`（`/api/dag/defs` 的真实存储）；按 name skip 不覆盖操作员编辑，失败仅 warn 不阻断启动；body 形态复用 `api::catalog::dag_definition`；code-quick 为 6 步带分支（triage→risks/api-impact，api-impact→client，verdict 汇 risks+client，report 收尾）。

## Brain 接缝

`/api/brain/plan-defs` 支持轻量 v3 计划，沿用现有版本表和幂等写入；旧图版本可读。v3 计划保存目标、默认命名输入、显式能力范围及轮次上限。启动可传准确 plan 引用，服务端解析后保存最终请求和来源版本，回执使用 run_id。通用执行 API 继续拒绝旁路创建 Brain。

`brain_runs/v3/catalog.rs` 共用目录适配与范围校验，保存、启动和每轮唤醒都验证选中的能力。注册能力解析真实目标定义，TODO 派发使用 spec 快照。`view.rs` 从持久化事件恢复轮次理由，从子执行 assignment 读取创建状态和历史能力元数据；根创建时保存能力范围的轻量描述。

旧决策树、Playbook 及 Project 的旧 Brain 执行模式返回迁移错误；历史查询保留。节点选点沿用 Fleet 规则，明确指定的子执行位置不会被根节点覆盖。

v3 根请求必须显式带 `schema_version: 3`。根节点持有 scheduler projection、generation 和模型上下文；control 只保存并转发执行索引、输入绑定和终态回执。子执行输入、输出、消息和 DAG 状态留在所属节点，详情按 `execution_id` 查询。非法能力或引用、无成功证据完成、创建失败及终态失败都进入阻塞/失败路径，不自动降级到通用 Agent。

`scheduler_wake_ack` 只确认收到的唤醒帧 generation；不以调用完成后的最新快照替代，防止快速子执行生成的新 Ready 轮次被旧回执提前确认。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
