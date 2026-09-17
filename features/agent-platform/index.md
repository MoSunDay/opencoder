Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# Agent 调度平台

Server/Node 调度、DAG 定义管理、执行查看与平滑发布。细节以代码为准。

## 定时调度

配置文件 `schedules.json` 声明式 cronjob：`id/cron/kind/target/params/enabled/timezone/overlap`，触发 agent/team/todos/dag/brain 既有执行入口，确定性执行 id `<kind>-<schedule_id>-<scheduled_for_ms>`；控制面统一调度，24h 追赶窗、密集 cron 折叠 `missed` 审计行、`overlap: skip` 防堆叠。新建条目首次扫描按同一 24h 窗口补跑最近一个到期 tick。台账 schema v26；查询 `GET /api/schedules`、`/api/schedules/:id/runs`（admin-only），CLI `opencoder-cli schedule list|runs`，Web 控制台 Agent 分类「定时任务」页（`spa/src/schedule/panel.jsx`，只读列表 + 触发历史 Drawer）。详见 [docs/agent-platform.md 定时调度](../../docs/agent-platform.md)。

## 相关
- [agents/control](../../agents/control/index.md) — 控制面
- [agents/worker](../../agents/worker/index.md)、[agents/node](../../agents/node/index.md)
- 发布与回滚命令见系统级发布流程。
