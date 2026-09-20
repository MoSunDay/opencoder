Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# Agent 调度平台

Server/Node 调度、DAG 定义管理、执行查看与平滑发布。细节以代码为准。

## 会话提交

Web 控制台的 operator 与 agent 会话都通过控制面的执行派发入口创建。节点首次
复制 Agent 资源快照可能耗时几十秒，因此 `Create` 派发使用 60 秒受理窗口；普通
节点查询和控制调用仍使用 15 秒超时。受理窗口内完成后直接返回 202 执行回执，避免
把已被节点接收的需求误报为 `node request timed out`。

## 定时调度

定时任务定义存控制面 libsql `schedules` 表（schema v27 事实源），`schedules.json` 降级为一次性 seed（表空才导入）；`id/cron/kind/target/params/enabled/timezone/overlap` 触发 agent/team/todos/dag/brain 既有执行入口（`params` 按类型消费：agent/team/todos 读 `prompt`——agent 触发即首轮消息、成功后回落追加进 how.md；dag 读 `args`——追加到每个 Wasm 步命令行；brain 读 `objective` 必填），确定性执行 id `<kind>-<schedule_id>-<scheduled_for_ms>`；控制面统一调度，24h 追赶窗、密集 cron 折叠 `missed` 审计行、`overlap: skip` 防堆叠。新建条目首次扫描按同一 24h 窗口补跑最近一个到期 tick。台账 schema v26、定义 v27；admin CRUD + 手动立即触发 `POST /api/schedules/:id/run`（绕过 enabled/overlap），查询 `GET /api/schedules`、`/api/schedules/:id/runs`（admin-only），CLI `opencoder-cli schedule list|runs`，Web 控制台 Agent 分类「定时任务」页（`spa/src/schedule/panel.jsx`）全功能管理（新建/编辑/启停/删除/立即触发/触发历史 Drawer）。详见 [docs/agent-platform.md 定时调度](../../docs/agent-platform.md)。

## 节点调度配置

节点并发与排队在节点侧配置：`GET/PUT /api/nodes/:id/scheduling`（admin-only，控制面经 Maintenance RPC 转发，见 [agents/control](../../agents/control/index.md)）读写 `max_runs`/`queue_order`，并可设置节点工作空间 `workdir`（绝对路径，空白即清空、恢复节点启动目录；持久化在节点 `scheduling.json`，重启后继续生效）。workdir 生效范围是该节点上非 brain 工作负载的会话工作目录、配置发现与会话归属（`workdir_hash`）；brain 工作负载仍用各执行目录的 `workspace`，节点自身数据目录与启动 workdir 不变（见 [agents/worker](../../agents/worker/index.md)）。多 runtime Host 不支持节点级 workdir：读接口返回 `workdir_supported:false`，配置带 workdir 返回 400，SPA 调度配置弹窗据此隐藏输入项。部署与 API 明细见 [docs/agent-platform.md](../../docs/agent-platform.md)。

## 相关
- [agents/control](../../agents/control/index.md) — 控制面
- [agents/worker](../../agents/worker/index.md)、[agents/node](../../agents/node/index.md)
- 发布与回滚命令见系统级发布流程。
