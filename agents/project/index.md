Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# project 模块

用户策展 goal→milestone→todo 与 Plan/Execute 双阶段执行。

## 关键路径
- `src/service.rs` — `ProjectService`（`OnceLock<Arc<Deps>>`）与活跃驱动注册。
- `src/runs.rs` — 接收/启动分离：`accepted_attempt`→`reserve_attempt`→`drive_reserved`。
- `src/executor/` — `resolve` 分发 agent/team/dag/brain；override 可免本地 brain。
- `src/executor/agent_drive.rs` — resume 同一会话推进；Agent/Harness/资源变更新建。
- `src/plan_gen.rs` — plan 会话生成方案，plan 固定 plan Agent。
- `src/trace/` — 逐次模型请求/响应、Codex 溯源与交付清单归档。
- `src/recover.rs` — stale/panic 收敛；恢复须显式触发，不自动重跑。
- `crates/store/src/project/overview.rs` — 纯投影，control/web 共用。
- store 表 v15 `project_*`、v23 里程碑放宽；`project_todo_runs` 版本留痕可取消。
- ProjectStore feature-gate `mysql`/`starrocks`；StarRocks 拒绝 Plan/Execute 写。

- `src/executor/playbook_drive.rs` + `playbook_step.rs` — 本地剧本编排：拓扑波次 JoinSet 并发、失败 `collapse_blocked` 折叠下游、每步 `kind=Step` 子 run 行（不写 todo 状态）；todos 目标本地拒绝（平台语义）。
## 边界
- 项目运行与 todos 自治 workflow 是不同入口。
- 共享外置 ProjectStore 的多运行时无分布式驱动租约。

## 相关
- [agents/control](../control/index.md) — 结构保存与派发。
- [agents/worker](../worker/index.md) — 节点执行与 run 读取。
- [agents/todos](../todos/index.md) — 另一执行入口。
- [agents/store](../store/index.md) — ProjectStore 接缝。
- [双节点浏览器验收](../../scripts/acceptance/project/README.md)。
- [Harness 验收](../../scripts/acceptance/harness/project.js)。
