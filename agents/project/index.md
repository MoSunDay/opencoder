Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# project 模块

## 平台归属

平台由 [control](../control/index.md) 保存项目结构，[worker](../worker/index.md) 使用本地 ProjectStore 运行本 crate。`project-<todo-id>` 固定归属节点，Plan、Act、新草稿 Plan 与恢复保持该节点；计划文本与 runs 在 Node，Server overview 查询后临时合并。`spawn_run_driver` 继承固定资源作用域。下面的 ProjectService 描述属于执行引擎，平台 Server 不初始化此运行时，也不连接旧 MySQL 项目库。

## 职责

`opencoder-project` 是用户手工策展的项目跟踪运行时：目标(goal) 1—N 里程碑(milestone) 1—N 待办(todo)，todo 可不挂里程碑（backlog）。每个 todo 的生命周期是「粗略草稿 → plan agent 生成完整实施方案 → 执行落地」：plan 阶段固定 plan agent；执行阶段执行器可选 agent/team/dag/brain（`executor_kind`，缺省 agent；team/dag 以 `executor_ref`+`executor_spec` 指定资源或内联定义，brain 可用 `executor_ref` 钉住能力，见「执行器」）。可反复执行，每次运行以 `project_todo_runs` 行留痕（version 自增；execute run 启动时落 plan_md 方案快照 + agent 输出 + 会话引用，plan run 只留 agent 输出）。

## 边界

- **不复用 todos crate 的编排**：那边是 LLM 自治 workflow（父会话调度 + candidate JSON 门禁 + 重试），这边只有 plan/execute 两种用户触发的直接驱动运行，没有状态机重试。
- **复用 session 直驱范式**（参考 `crates/todos/src/execution.rs`）：`SessionState` + `opencoder_session::run/resume` + `spawn_event_flusher` 事件落库——运行会话可在「会话交互」页完整回放。
- 会话/消息仍走 `Arc<dyn Store>`（libsql）；**项目数据走独立 `Arc<dyn ProjectStore>`** 接缝（默认 libsql 同实例，可选 feature-gate mysql/starrocks，见 [agents/store](../store/index.md)）。Node 的 `opencoder-agent` 固定使用节点 `runtime.db` 的 libsql；旧 Web/CLI/TUI 进程才会读取可选的外置 Project backend。
- todo 状态机服务层独占：`draft →(plan 成功) planned →(execute) running → done|failed`；execute 取消 → 回 `planned`；可从 done/failed/planned 重复 execute（新 version）。Web PATCH 不暴露 status/plan_md。

## 执行器

todo 的执行器维度在 `src/executor/`：`resolve` 纯解析（override 优先；Agent 直驱、Team/Dag 要求 `executor_ref`/`executor_spec` 至少其一、Brain 不在此解析）+ 唯一派发入口 `drive`。四个子驱动共享同一 run 行生命周期（claim → drive → close_run → todo 回写）、同一取消注册表与 panic/stale 兜底（`recover.rs`），差异只在「谁干活」：

- `agent_drive`：act 会话直驱（既有范式），新建或 resume 同一 `active_session_id`。
- `team_drive`：`LocalTeamDispatcher` 每 ask 一会话；物化团队名 `project-{净化 todo id}`，team_root 缺省 `data_dir_for(workdir)/team`；FINISH_* 映射 run/todo 终态。
- `dag_drive`：`Uplink::for_local_dag` 本地执行，事件批量写 session_events；workflow_root `<data_dir>/workflow`，run_id 即项目 run id（`prun-<ULID>`）。
- `brain_drive`：`resolve_brain` 运行时解析为具体 agent/team/dag（拒绝 brain→brain 嵌套），解析在 claim 之前，失败不留半启动状态。

run 行记录**解析后**的 `executor_kind`：brain todo 执行时解析为实际 agent/team/dag，并带 `capability_id`/`plan_id` 溯源与 `output_ref`（dag=工件根路径 / team=topic id）。

内联 spec 纯类型（`TeamSpec`/`BrainRoutes`/`validate_spec`）在 `opencoder-store`（`project_executor_spec`，dag 校验委托 `opencoder-dag`）——控制面共享的 API 校验不能链接本 crate 执行引擎（P0 拆分）；本 crate 以 `executor::spec` 再导出。

Brain 双通道：本地 web 由 `ProjectService::init` 第 5 参注入 `opencoder_brain::Runtime`（situation=标题+草稿+方案，`dispatch_or_plan` 路由；`executor_spec` 缺省 BrainRoutes {agent, "act"}）；平台由 control `brain_preresolve` 预解析（fleet capability_target 绑定）经执行 input 的 `"brain"` 键下发，worker 读 `record.result["brain"]` 优先、回落 `input["brain"]`（预检经 `mirror_result_overrides` 同序 result-first）→ `start_execute_with(todo_id, ExecutorOverride)`（serde 字段 `ref`）；claim 前解析结果以 `BrainHandoff { override_, trace }` 随 `executor::drive` → `brain_drive` 透传，驱动内重解析直接采纳 override 纯函数分支——节点 `init` brain=None 不本地路由但**可执行**预解析 brain todo（`resolve_brain`/`resolve` 均拒绝 brain→brain override）；worker 预检 `project_preflight_agents` 按 kind 聚合 agent 清单（team/dag 取内联 spec，无 spec 惰性解析不预检）。

## 关键抽象

- `ProjectService`（`service.rs`）：`OnceLock<Arc<Deps>>` 惰性初始化（`new()` 同步便宜，`init()` async 给后端构建留空间；第 5 参 `brain: Option<opencoder_brain::Runtime>`）；`spawns: Mutex<HashMap<run_id, CancellationToken>>` 取消注册表；`start_plan/start_execute → run_id`（`start_execute_with` 额外接受控制面 brain 预解析覆盖）、`cancel(run_id) → bool`、`overview() → {goals:[{…, milestones:[{…, todos:[…]}]}], backlog:[…]}`。
- `plan_gen::drive`（plan）/ `executor::drive`（execute，按解析结果派发子驱动）：spawn 出的后台驱动。plan 每次新建 plan-agent 会话（task_type=`project`）；agent 执行器 **新建或 resume** 同一 `active_session_id`（持续推进），watermark 后取最后一条 assistant 文本为 `output_md`。
- 取消语义：`session.cancel = Some(token)`；`cancel()` 克隆并触发注册令牌，驱动完成输出与回放归档后才通过 `forget_spawn` 摘除标记。重复取消与 stale 扫描不能把仍在收尾的驱动当作丢失。Plan/Agent Act 通过 `attempt_outcome` 优先判定取消，保留已有部分输出，避免工具步骤中的 assistant 文本把取消误标为 done；Execute 取消后 todo 回到 `planned`。
- 崩溃兜底：驱动任务 panic 由监控任务收敛（run→failed、execute 的 todo→failed、plan 不动 todo）；进程重启丢失驱动留下的 running run 由 `overview()` 读路径机会式清扫（`STALE_RUN_GRACE_MS`=5 分钟 grace 后 run→failed、其 running todo→failed）。收敛均为条件 CAS——run 已终态不改写标签（但 todo 仍 Running 时补收敛，不悬死）。stale 自愈不依赖总览读路径：execute 前置守卫对「不在注册表且超 grace」的 stale plan 行机会式收敛放行（grace 内未注册行保守拒绝，兜住并发 start_plan 的 create→注册窗口）；`cancel` 对 lost-driver 的 running 行即时收敛（run→cancelled、execute 的 todo→planned 回退，不等 grace——run_id 仅在令牌注册后对外可见，无窗口误伤）。**单进程独占 store 假设**：sweep 以本进程内存注册表判定驱动存亡，libsql 本地嵌入天然满足；mysql/starrocks 共享 DSN 的多进程部署会误收敛活跃 run（真实 driver 随后终态写回可自愈，仅留 failed 噪声）——多进程化前需引入持有者标记。
- `context.rs`：纯函数 prompt 组装（`plan_prompt`/`execute_prompt`，含 goal/milestone 上下文与 resume 版本提示）。
- `client_for(config, override)`：测试注入 `MockChatClient` 的接缝，生产走 `resolve_endpoint + ChatClient`。

## 主流程

1. web `POST /api/project/todos/:id/plan` → `start_plan`：建 run(kind=plan, version=n, running) → spawn：建会话 → `run()`+flusher → 成功即 `todo.plan_md=output, status=planned`，run done；失败 run failed（todo 状态不动）。
2. `POST /api/project/todos/:id/execute` → 校验有 plan 且非 running（plan run 进行中拒绝执行——plan/execute 互斥正向）→ Store 在一个事务中条件 claim todo 并创建 run(kind=execute, plan_md=启动时方案快照)，并发仅一方成功，run 插入失败整体回滚，不会出现 Running todo 无 run → claim 前先解析执行器（`resolve`/`resolve_brain`，brain 缺运行时即拒）→ spawn：缺省 agent 直驱（resume 或新建 act 会话）→ `run()` → 成功 todo done；取消 todo 回 planned；失败 todo failed。plan 收尾的 todo 回写同为条件 CAS（`commit_plan_output`）：todo 被 execute 抢先 claim（Running）时丢弃回写，方案仍留痕于 run 行。libsql 与 MySQL 支持该原子执行入口；StarRocks 不支持跨表事务，execute 在写入前明确失败，CRUD/查询与 plan 留痕仍可用。
3. `POST /api/project/runs/:rid/cancel` → token cancel → 驱动任务收敛终态并从 spawns 注销。

## 测试

- 单元：`src/service_tests.rs`（service.rs 的单元测试，独立文件以守单文件行数上限）：prompt 组装 4 例；service 未初始化/未知取消 2 例；plan 收尾条件回写（Running 让路 / 非 Running 落 Planned / 行缺失）3 例；panic 收敛（终态标签不打花、run 已 Done 但 todo 悬 Running 补收敛）2 例 + plan 进行中拒绝执行 1 例。
- 集成 `crates/project/tests/plan_and_execute.rs`：plan 生成回写 plan_md、执行建会话 + 二次执行 resume 同会话、中途取消回 planned、无 plan/running 拒绝、execute 启动快照 plan_md、原子 claim 失败无状态变化、overview 树形；Store 独立覆盖并发唯一 winner 与 INSERT 失败事务回滚。
- 集成 `crates/project/tests/executor_team_dag_brain.rs`（真 store + MockChatClient）：内联 dag spec 本地工作流执行 + 工件落盘、内联 team spec 本地讨论收敛、brain 钉住能力经路由表落到已登记 dag、节点缺 brain 运行时在 claim 之前拒绝，节点缺 brain 运行时在 claim 之前拒绝、brain override 在节点上驱动成功（留痕 kind/capability/plan 与 override 同源）+ brain→brain override claim 前拒绝、dag 解析失败不留悬挂 session_id，共 8 例。
- web 契约 `crates/web/tests/web_project{,_runs}.rs`（真签名 build_app）：CRUD、plan/execute/runs 生命周期、overview、409/404 形状；执行器三字段创建/补丁与 run 暴露解析后 kind 的用例在 `web_project.rs`；SPA 执行器选择/展示见 `crates/web/spa/src/project/todosTab.dom.test.jsx`（7 例）。
