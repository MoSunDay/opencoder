Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# Agent 调度平台

Server/Node 调度、DAG 定义管理、执行查看与平滑发布；细节以代码为准。

## 执行准入与并发

- 节点磁盘可用块至少 10%、可用 inode 至少 20% 才接收新执行；容量读取失败或零容量拒绝准入，已有任务继续完成。
- DAG 定义的 `max_concurrency` 可配置为 1–30，缺省 4；画布和 JSON 编辑保存同一字段。每次运行冻结定义，修改定义不会改变在跑任务的上限。
- [原生 DAG](../dag/index.md) 在一个节点的一次共享容器内执行，源工作区只读，文件修改落在节点本地写层；静态与动态运行都需要节点声明容器能力。
- 兼容发布由当前 Server 信号触发独立作业；新任务转入新版本，既有任务保留所属 runtime。
- 平滑切换受理与连续调度的验收上限为 30 秒；公共入口不得出现失败请求。详细门槛与维护切换要求见 [发布说明](../../docs/smooth-release.md)。
- Host 的新任务就绪状态由活动 Runtime 决定；已退休 Runtime 的休眠库存仍用于历史执行索引，但其中的旧资源错误不阻断新任务。候选 Host 在切换前以自身与 Runtime 状态验证，切换后再由公共入口验证。
- 节点冻结或复开不会启动已休眠 Runtime；查询原任务或继续执行时才唤醒，并在放行请求前同步当前准入模式。

## 维护升级

- 数据格式 1 到 2、项目 schema v31 到 v32 使用一次维护窗口，关闭公共业务入口为 503，保留节点通道直到已接纳任务排空。
- 停止全部写入服务并保存一致性备份后安装固定配置，先升级独立资源服务，再建立原生二进制与工作区的只读 NFS 挂载；已有 Agent 只读挂载是预检条件。
- 候选 Server 的迁移启动意图持久化后，旧库备份不可用于回滚；即使启动回执丢失或公共写入尚未开放，也必须继续同一候选或用新 schema 兼容版本修复。
- 中断续跑使用同一候选、备份和配置快照。私有验证通过才开放接入；完成后的发布与回滚只接受数据格式 2，旧格式版本不能重新激活。
- 维护范围限已完成首次迁移的单机 libsql 安装；远程写入节点及未经真实恢复验收的外部数据库提前拒绝。具体操作见 [发布说明](../../docs/smooth-release.md)。

## 调度监控

- 管理员可通过 `GET /api/metrics/scheduler` 读取调度总览，Prometheus 使用独立 Bearer 凭据抓取 `GET /metrics`；该凭据不能访问其他接口，且不能与管理员凭据相同。Grafana“OpenCoder 调度总览”区分派发失败与执行失败，展示节点容量、运行数及下一次触发时间；指标不带任务 ID 标签。
- 扫描与派发计数随 Server 进程重启归零；日程最近状态来自持久化记录。`active_executions` 包含未终态执行索引，可能有历史遗留；实际设备运行数以 `node_active_runs` 为准。

## 页面状态与资源管理

- 身份确认前不展示管理页面；身份读取失败可重试，401 返回登录。读取失败与空列表、已停止、默认配置分别显示，切换页面或资源后旧请求不能覆盖新内容。
- Agent 配置展示 Agent、二进制和源工作区三个 NFS 的实际状态与路径；源工作区路径只读，停止导出须确认。
- 定时任务支持创建、编辑、启停和确认手动触发；历史记录打开原执行，删除定义不删除已有触发历史。
- [全站 UI 验收约定](../../rules/05-ui-acceptance.md) 同时检查全部注册页面、四种屏宽与 Server TUI；截图不能代替真实调度和完整操作链路。

## Operator 执行隔离

- 每个 Operator 执行获得独立的 HOME（冻结配置快照，0600）与 workspace；bash 工具 cwd 即 workspace，resume 重建同一对目录。
- Operator 会话创建前可选择 OpenCoder 或 Codex，并注入逐项环境变量；Codex 执行继承托管 Harness 设置，显式注入值覆盖同名托管 env，HOME 始终指向该 Operator 的隔离目录。启动选择与注入 env 随会话固定，续会话沿用。
- 窄屏会话页将节点与会话列表置于输入区上方，Operator 启动配置与发送输入均可操作。
- Operator 配置来自节点数据根下的专用平面（首个执行引导一次后冻结），交互端（TUI/CLI）后续保存的配置与新增的全局技能包不再影响 Operator 执行；执行技能池 = 平面包 + 内置技能。
- 会话按创建时打上的 `kind` 泳道隔离（`operator`/`agent`/`team`/`dag`/`todos`/`project`/`brain`）；默认会话清单不显示 operator 泳道。

## TUI 任务入口

- 配置 `opencoder_server.enabled=true` 和 Server `url` 后，`/agent` 列出 Server 能力库中的 Agent/Operator；鉴权沿用 `OPENCODER_SERVER_TOKEN`。默认关闭时只有 `self`。`@` 按普通文本发送。
- `/agent self` 创建空的本地任务；`/agent <能力 ID>` 创建新的远端任务，使用该能力的上下文。`/task`（`/tasks`、`/t`）列出并恢复本地任务与远端书签，切换时只展示目标任务的内容。
- 首轮与后续输入都由 Server 执行；连续对话、队列输入和引导沿用同一个执行 ID。首轮准入失败可按原 ID 重试，恢复任务不会重新创建执行。
- 文本、工具、问题和历史内容复用 TUI 的聊天展示。远端用户消息显示原文，内部执行前缀不作为用户内容回显；断线后从已消费位置继续读取。
- 切换和退出 TUI 只断开远端连接，Server 任务继续执行；`/stop` 或双 Esc 显式中断任务。
- Operator 可以使用 Server 注册的 Codex 包装器。TUI 的本地执行器、模型和环境选择不覆盖它；仅使用远端能力时无需本地模型凭据。
- 远端任务的本地模型、执行器和会话改写命令显示不可用；连接、权限或书签错误明确显示，不启动本地模型。

实现索引见 [tui](../../agents/tui/index.md)、[control](../../agents/control/index.md)、[worker](../../agents/worker/index.md)。

## DAG 私有任务文件

- `POST /api/executions` 的 `private_context` 承载有期限的执行专属文件，与公开 `input` 分离；节点能力接口为 `GET /api/nodes/{id}/execution-capabilities`。
- 调用方冻结 `image_digest`（运行中节点执行文件 SHA256）、`definition_sha256` 和 `expires_at_ms`。节点准入及恢复核验执行文件，私有文件变化不能复用旧执行 ID。
- 节点使用 0700 私有目录和 0600 文件，共享 runc 容器只读挂载 `/run/opencoder-task`；模型仅接收目录路径，系统不自动归档私有目录。DAG rootfs 安装脚本包含 Python 标准库运行时。
- 边界与回归映射见 [私有任务文件](../changelog/2026-09-22/private-dag-task-files.md)。

## 相关

- [协议与 API 明细](../../docs/agent-platform.md)、[平滑发布](../../docs/smooth-release.md)
- [动态 DAG 步骤](../../docs/dag-dynamic.md)
- [agents/control](../../agents/control/index.md) — 控制面与节点调度
- [agents/worker](../../agents/worker/index.md)、[agents/node](../../agents/node/index.md) — 节点执行与出站连接
