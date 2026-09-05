Commit: 6cb5ea1

# project 模块

## 职责

`opencoder-project` 是用户手工策展的项目跟踪运行时：目标(goal) 1—N 里程碑(milestone) 1—N 待办(todo)，todo 可不挂里程碑（backlog）。每个 todo 的生命周期是「粗略草稿 → plan agent 生成完整实施方案 → act agent 执行落地」，可反复执行，每次运行以 `project_todo_runs` 行留痕（version 自增；execute run 启动时落 plan_md 方案快照 + agent 输出 + 会话引用，plan run 只留 agent 输出）。

## 边界

- **不复用 todos crate 的编排**：那边是 LLM 自治 workflow（父会话调度 + candidate JSON 门禁 + 重试），这边只有 plan/execute 两种用户触发的直接驱动运行，没有状态机重试。
- **复用 session 直驱范式**（参考 `crates/todos/src/execution.rs`）：`SessionState` + `opencoder_session::run/resume` + `spawn_event_flusher` 事件落库——运行会话可在「会话交互」页完整回放。
- 会话/消息仍走 `Arc<dyn Store>`（libsql）；**项目数据走独立 `Arc<dyn ProjectStore>`** 接缝（默认 libsql 同实例，可选 feature-gate mysql/starrocks，见 [agents/store](../store/index.md)）。
- todo 状态机服务层独占：`draft →(plan 成功) planned →(execute) running → done|failed`；execute 取消 → 回 `planned`；可从 done/failed/planned 重复 execute（新 version）。Web PATCH 不暴露 status/plan_md。

## 关键抽象

- `ProjectService`（`service.rs`）：`OnceLock<Arc<Deps>>` 惰性初始化（`new()` 同步便宜，`init()` async 给后端构建留空间）；`spawns: Mutex<HashMap<run_id, CancellationToken>>` 取消注册表；`start_plan/start_execute → run_id`、`cancel(run_id) → bool`、`overview() → {goals:[{…, milestones:[{…, todos:[…]}]}], backlog:[…]}`。
- `plan_gen::drive` / `execute::drive`：spawn 出的后台驱动。plan 每次新建 plan-agent 会话（task_type=`project`）；execute **新建或 resume** 同一 `active_session_id`（持续推进），watermark 后取最后一条 assistant 文本为 `output_md`。
- 取消语义：`session.cancel = Some(token)`；硬取消中止在途流后 `run()` 可能返回 `Ok(())` 且无新 assistant 消息——两种路径都收敛为 run `cancelled` + todo 回 `planned`。
- 崩溃兜底：驱动任务 panic 由监控任务收敛（run→failed、execute 的 todo→failed、plan 不动 todo）；进程重启丢失驱动留下的 running run 由 `overview()` 读路径机会式清扫（`STALE_RUN_GRACE_MS`=5 分钟 grace 后 run→failed、其 running todo→failed）。收敛均为条件 CAS——run 已终态不改写标签（但 todo 仍 Running 时补收敛，不悬死）。stale 自愈不依赖总览读路径：execute 前置守卫对「不在注册表且超 grace」的 stale plan 行机会式收敛放行（grace 内未注册行保守拒绝，兜住并发 start_plan 的 create→注册窗口）；`cancel` 对 lost-driver 的 running 行即时收敛（run→cancelled、execute 的 todo→planned 回退，不等 grace——run_id 仅在令牌注册后对外可见，无窗口误伤）。**单进程独占 store 假设**：sweep 以本进程内存注册表判定驱动存亡，libsql 本地嵌入天然满足；mysql/starrocks 共享 DSN 的多进程部署会误收敛活跃 run（真实 driver 随后终态写回可自愈，仅留 failed 噪声）——多进程化前需引入持有者标记。
- `context.rs`：纯函数 prompt 组装（`plan_prompt`/`execute_prompt`，含 goal/milestone 上下文与 resume 版本提示）。
- `client_for(config, override)`：测试注入 `MockChatClient` 的接缝，生产走 `resolve_endpoint + ChatClient`。

## 主流程

1. web `POST /api/project/todos/:id/plan` → `start_plan`：建 run(kind=plan, version=n, running) → spawn：建会话 → `run()`+flusher → 成功即 `todo.plan_md=output, status=planned`，run done；失败 run failed（todo 状态不动）。
2. `POST /api/project/todos/:id/execute` → 校验有 plan 且非 running（plan run 进行中拒绝执行——plan/execute 互斥正向）→ 条件 CAS 把 todo 置 running（`claim_todo_running`，单条 UPDATE 关死并发重复执行的 TOCTOU）→ 建 run(kind=execute, plan_md=启动时方案快照)；建 run 失败会条件回滚 claim 前状态（不滞留 Running）→ spawn：resume 或新建 act 会话 → `run()` → 成功 todo done；取消 todo 回 planned；失败 todo failed（不会滞留 running）。plan 收尾的 todo 回写同为条件 CAS（`commit_plan_output`）：todo 被 execute 抢先 claim（Running）时丢弃回写，方案仍留痕于 run 行。
3. `POST /api/project/runs/:rid/cancel` → token cancel → 驱动任务收敛终态并从 spawns 注销。

## 测试

- 单元（内联）：prompt 组装 4 例；service 未初始化/未知取消 2 例；plan 收尾条件回写（Running 让路 / 非 Running 落 Planned / 行缺失）3 例；panic 收敛（终态标签不打花、run 已 Done 但 todo 悬 Running 补收敛）2 例 + plan 进行中拒绝执行 1 例。
- 集成 `crates/project/tests/plan_and_execute.rs`：plan 生成回写 plan_md、执行建会话 + 二次执行 resume 同会话、中途取消回 planned、无 plan/running 拒绝、execute 启动快照 plan_md、create run 失败回滚 claim（故障注入包装）、overview 树形。
- web 契约 `crates/web/tests/web_project{,_runs}.rs`（真签名 build_app）：CRUD、plan/execute/runs 生命周期、overview、409/404 形状。
