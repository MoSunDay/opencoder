# Agent 调度平台

`opencoder` 保留 CLI/TUI；初始部署可以使用两个进程：`opencoder-server` 负责 Web、全局定义、大脑和调度，`opencoder-agent` 负责节点上的 agent loop。Server 不创建本地会话、不执行团队或工作流，也不链接 Python VM/runc。

## 部署

```bash
cargo build --workspace

# 凭据由管理员单独创建并以 0600 文件提供；进程不会生成或打印 token
opencoder-server --host 127.0.0.1 --port 8080 \
  --workdir /etc/opencoder/server --data-dir /var/lib/opencoder-server \
  --token-file /run/credentials/opencoder-server.service/token

# 每个执行节点使用独立工作目录和持久化目录；示例布局为
# /data00/<kind>/<id>/execution.json
opencoder-agent --remote https://opencoder.internal.example --name worker-a \
  --workdir /etc/opencoder/agent --data-dir /data00 \
  --token-file /run/credentials/opencoder-agent.service/token
```

Server 与 Node 都要求 `--token`、`--token-file` 或既有 `OPENCODER_SERVER_TOKEN` 三者之一；生产服务使用 `--token-file`，token 不进入 URL、进程参数或日志。节点主动建立携带 Bearer 凭据的 WebSocket，无需开放节点 HTTP 入站端口。节点工作目录中的 `opencoder.json` 配置执行用的模型与凭据；Server 配置中的模型用于大脑。大脑未配置模型时仍可管理节点及派发普通任务，调用大脑时返回具体配置错误。

仓库提供 [Server unit](../deploy/systemd/opencoder-server.service)、[Agent unit](../deploy/systemd/opencoder-agent.service) 和 [Nginx HTTPS/WSS/SSE 样例](../deploy/nginx/opencoder.conf)。部署前分别创建 `/etc/opencoder/server.token` 和 `/etc/opencoder/agent.token`，写入同一 Bearer token 并设为 `0600`，owner 必须是对应服务用户；unit 只把文件路径传给 `--token-file`，不会把 token 内容放入参数、环境变量或日志。样例 Agent 以 root 运行以支持当前真实 runc 路径，并使用 `KillMode=mixed`，保证停止时只有 Agent 先收到 TERM，已有任务可自然 drain；若禁用 runc，可在验证目录、进程树和 NFS 权限后改用专用用户。内网 CA 的完整证书链必须安装到每个 Node 和管理员浏览器的系统信任库；Agent 的 HTTP 与 WebSocket 客户端都使用系统 CA，不提供跳过证书校验的降级开关。

`--max-runs` 限制节点同时承接的顶层执行，默认是可用 CPU 数向上取整；`--no-dag` 禁止该节点接收 DAG。节点 ID 持久化在 `data-dir/node-id`，同一目录有进程锁，不能同时启动两个 worker。更换展示名称不会更换 ID。

打开 Server 的 Web 页面，输入连接凭据，即可使用节点、会话、项目、DAG、TODO、资源、团队、全部执行和大脑入口。

## 数据归属

| 数据 | 持久化位置 |
| --- | --- |
| 节点注册信息、agent/team/DAG 定义、大脑能力与规划、项目结构 | Server |
| 每条执行的索引 | Server，仅 `id`、`created_at`、`kind`、`node_id`、`status` |
| 用户输入、消息、工具调用、事件、team topic、工作流状态与检查点 | 所属 Node |
| DAG 产物、项目 Plan 文本与执行过程、子 agent 会话 | 所属 Node |
| prompts、skills、tools、memory 资源 | Server 发布，经只读 NFS 共享；执行时在 Node 固定快照 |

普通 agent、子 agent、team、DAG、TODO workflow 和项目 Plan → Act 始终在一个节点闭环。Server 通过 ID 查索引，再向所属 Node 获取明细、事件或产物；列表需要名称等额外字段时临时查询节点，不落库缓存。节点离线时查询明确失败，不用旧副本冒充当前明细。

平台使用独立新库：Server 的 `data-dir` 包含 `control.db`、`definitions.db` 和持久化的 `admission.json`；省略该参数时兼容使用当前工作目录数据域下的 `server-v2/`。Node 的 `data-dir` 包含 `runtime.db`、`admission.json`、`scheduling.json` 和 `<kind>/<id>/execution.json` 及其资源、team、DAG 状态。未指定 Node `data-dir` 时使用当前工作目录数据域的 `node-v2/`。旧 daemon/CLI 数据不迁移、不清空；需要迁移旧 Node 布局时显式运行 `opencoder-agent --data-dir <目录> storage migrate-layout`。平台项目定义固定使用新 Server 库，不读取旧 MySQL 项目库。

## 调度和恢复

调度分数为 `(活跃 agent loops + 待确认分配数) / 可用 CPU`，优先最低值。CPU 包含容器 quota；统计真实运行的父/子 agent、团队成员、工作流 agent 和维护 agent，空闲会话不占 loop。节点还必须在线、心跳新鲜、资源可用且支持执行类型。优先选择有空余容量的节点；全部满载时选择等待数较少的可接受节点，把超额任务交给该 Node 持久化排队。

节点每 5 秒报告心跳，并在 loop 进入/退出和任务状态变化时立即报告快照与索引。Server 在发出任务前记录归属并预留容量，收到节点确认后释放预留，避免并发请求集中投向同一空闲节点。指定 `node_id` 时只检查指定节点；节点不满足条件则报错，不改派其他节点。

Node 在返回接受前同步持久化任务、资源和 Harness 配置快照。旧单进程节点可设置最大顶层并发数（1–65535）、FIFO/LIFO 与可选工作空间 `workdir`（绝对路径，空白清空恢复节点启动目录；生效范围覆盖该节点非 brain 会话的工作目录、配置发现与会话归属，brain 工作负载仍用各自执行目录的 `workspace`；多运行时宿主不支持，配置返回 400）；`scheduling.json` 保存配置，优先于重启时的启动默认值。超额任务以 pending 等待，释放容量后按配置顺序启动；降低上限不打断正在运行的任务。创建请求携带稳定 ID：同一 ID、相同输入重复提交不重复执行；不同输入返回 409。网络超时后归属仍保留，客户端必须用原 ID 重试，Server 不猜测任务是否接受而改派。

断线只影响传输，已接受的本地工作继续。Node 重启将原先运行中的执行标记 `interrupted`，由用户显式在原节点恢复；已接受但尚未开始的持久化 pending 队列继续等待，冻结节点在复开后才继续调度。DAG 恢复跳过已成功写入检查点的步骤。项目执行 ID 为 `project-<todo-id>`，Plan、Act 和新一轮 Plan 保持相同节点。节点存储出现持久化错误时上报不可调度，须处理存储故障后重启节点。项目页面与全部执行详情中的 Plan 均由 Server 解析当前草稿；Node 在忙碌及资源预检通过后才保存该轮快照，拒绝请求不修改之前的运行记录；容量不足会保存并排队，等待期间 Project 运行不会被失联清理误判。

## 定时调度

定时任务定义自 schema v27 起持久化在控制面 libsql `schedules` 表（事实源）；`schedules.json`（server workdir 的 `.opencoder/` 域文件，或全局 `~/.opencoder/`）降级为一次性 seed——仅表空时全量导入（非法条目告警跳过，不阻断启动），此后文件改动不再回灌（删除不会在重启时复活），但 `scan_interval_secs` 永远以文件为准（默认 15s，最小 1s，调度循环热读）。Server 控制面内置 cron 调度器（无定义即空转，配置读取失败或条目非法仅告警跳过，不影响其余任务）。字段：`id`（1–40 字符，字母/数字/`-`/`_`，用于确定性执行 ID）、`cron`（5 字段为分 时 日 月 周；6/7 字段保留秒位）、`timezone`（仅固定偏移如 `+08:00`）、`enabled`（默认 true，关闭的条目不做校验）、`kind`（`brain`/`team`/`todos`/`agent`/`dag`）、`target`、`params`（按 kind 消费：agent/team/todos 读 `prompt`——agent 触发时作为首轮消息提交并经回落机制补进 how.md，dag 读 `args`，brain 读 `objective`/`inputs`/`mode`/`plan`）、`overlap`（`skip` 默认 / `allow`）、`node_id`（可选钉住节点）、`scan_interval_secs`（扫描间隔，默认 15s，最小 1s）。

触发复用既有入口：agent/team/todos/dag 走 `POST /api/executions` 同一条提交链路，brain 走 brain run 创建链路；`params` 支持时间模板 `{{now[±N<单位>][:格式]}}`（单位 s/m/h/d/w，缺省 RFC3339，另有 `unix`/`unix_ms`），在触发时刻渲染为执行 input；dag 的 `args`（字符串，可选）在触发时追加到每个 Wasm 步的命令行（空白切分成 argv，幂等不重复追加）。每次触发获得确定性执行 ID `<kind>-<schedule_id>-<scheduled_for_ms>`：同一 tick 重复提交幂等收敛，不重复执行。Server 停机重启后仅补跑最近一个错过的 tick，更早的记为 `missed`（24 小时补跑窗口）；提交失败的 tick 在 1 小时内重试，超窗后等待下一个 tick。新建条目首次扫描时没有历史台账，基线退化为 24 小时窗口起点：窗口内最近一个到期 tick 会在首次扫描立即补跑，更早的记为 `missed`。`overlap: skip` 时上一轮触发对应的执行未到终态则本轮不触发，`allow` 无条件触发。

触发历史持久化在 `schedule_runs` 表（schema v26 起），按 `(schedule_id, scheduled_for_ms)` 主键覆盖写；定义持久化在 `schedules` 表（schema v27，主键 `id`，`job` JSON + created_at/updated_at，upsert 保留 created_at）。`GET /api/schedules` 列出全部定义并附最近一次触发与下一次触发时刻，`GET /api/schedules/:id/runs?limit=` 返回倒序历史；admin CRUD：`POST /api/schedules` 创建（缺省 id 自动生成 `schedule-<ULID>`，重名 409，非法 body 400）、`PUT /api/schedules/:id` 全量更新（404 未知 id，created_at 保留）、`PATCH /api/schedules/:id` 仅启停（`{"enabled": bool}`，重校验整个定义——坏 cron 的停用条目无法被直接启用）、`DELETE /api/schedules/:id` 删除定义（触发历史保留可查）、`POST /api/schedules/:id/run` 手动立即触发（绕过 enabled 与 overlap，属显式操作员动作）。全部端点 admin-only；CLI 对应 `opencoder-cli schedule list` 与 `opencoder-cli schedule runs <id>`；Web 控制台「定时任务」页（`spa/src/schedule/panel.jsx`）提供新建/编辑/启停/删除与手动触发的全功能管理。

## NFS 资源共享

Web「Agent 配置」按 Agent 名称打开配置抽屉，直接查看和编辑 Prompt（Soul、How、Output）、Skills 目录及附件、Tools 文件和 `memory.md`。新建只填写名称与执行方式，资源首次保存自动创建并绑定；Prompt 至少一部分非空。历史版本位于各页签的「历史版本」，恢复会生成新版本。文本支持编辑和预览，二进制支持下载与上传替换，工具保留执行权限；切换页签保留草稿，关闭或刷新未保存内容会提示。读取错误禁止覆盖保存，保存失败保留输入。内置 Agent 显示实际 Prompt、工具限制及已有 Agent 级资源，资源只读，缺少的类别标注未配置。

旧资源默认共享。按 Agent 保存或恢复时，首次编辑会复制完整资源内容和版本历史到带 `owner_agent` 的独立资源，然后原子切换当前 Agent 的引用；后续保存递增版本，不改变其他 Agent。写入和引用变更使用同一资源根文件锁；资源基线不匹配返回 409。完整目录落盘后才发布，未编辑文件保持字节与权限，越界路径和符号链接被拒绝。`tools_scope=all` 保留共享工具，当前 Agent 的工具优先，排除其他 Agent 的专属工具。

Codex 的二进制、模型、推理、权限参数和 env 在 Harness 管理中统一保存，正在运行、排队及续聊的会话保持已接受的参数。共享内容限于 agent 定义及 prompts/skills/tools/memory，不共享 runtime DB、对话、项目运行记录或 DAG 产物。

使用页面提供的挂载命令，并保留 `ro`：

```bash
mount -t nfs -o ro,vers=3,tcp,port=<port>,mountport=<port>,nolock,soft,retrans=1,timeo=50,actimeo=0,lookupcache=none server:/ /mnt/opencoder-agents
```

在 Node 的 `opencoder.json` 中设置 `agent.agents_dir` 为 `/mnt/opencoder-agents`。显式配置该路径时 Node 校验 Linux 挂载表，要求可读的只读 NFS；未挂载、可写挂载或本地目录都返回资源错误，不静默使用本地资源替代。

NFS 资源服务的导出支持完整深层资源路径，短路径句柄保持兼容，长路径句柄在导出重启后可恢复。目录读取失败明确返回错误，不以漏文件的列表代替成功。

DAG wasm 模块池是第二路只读导出：Server 侧 `dag.nfs.enabled` 开启（默认 `127.0.0.1:2050`、只读，导出根为 `dag.wasm_dir` 或数据目录默认 `<data>/dag/wasm`）。节点用同样的只读参数挂载后，在 `opencoder.json` 设置 `dag.wasm_dir` 指向挂载点：

```bash
mount -t nfs -o ro,vers=3,tcp,port=2050,mountport=2050,nolock,soft,retrans=1,timeo=50,actimeo=0,lookupcache=none server:/ /mnt/opencoder-dag-wasm
```

该路径不做挂载表强制校验，但节点必须以 `ro` 挂载并只读加载：未配置时 wasm 模块维持 out-of-band 投放；配置后节点受理 DAG 时把 spec 引用的池模块冻结进 `<workflow_root>/_modules/`（`tool.wasm` 取 current，`tool@v3.wasm` 取显式版本），池缺名或缺该版本视为 out-of-band 跳过，导出内容损坏（sha256 不符）则拒绝受理。WASM 和 Agent 任务都只执行节点本地快照，不向 NFS 写入或直接依赖 NFS 运行。发布/回滚只影响之后的新受理，不影响已接受 run；本机部署可参照 agents 挂载单元模板复制第二路挂载。

本机部署可使用 `scripts/platform/systemd/` 的只读挂载单元及 Agent 依赖配置；跨主机部署调整 `What` 为实际 Server。每个挂载点只保留一个挂载，关闭目录与属性缓存使资源发布及时对新任务生效。回滚不支持长句柄的旧 Server 时，先停止依赖该挂载的 Node，再受控重新挂载。

每次新执行复制当前版本到节点资源快照，包含实际文件。显式资源源路径消失或复制失败时拒绝接受，不生成空快照；只有未配置资源的内置 Agent 可以使用空资源池。资源后续发布、回滚或移除不会改变已接受的执行。缺失引用、不可读资源和版本内符号链接在接受前报错。已有执行的继续或恢复使用已固定快照，NFS 断开不阻止这些操作；新执行需要共享目录可用。仅使用内置 agent 时可以不配置共享目录。

## 团队与大脑

普通团队定义包含 captain、成员 agent 和职责；团队启动后所有成员在一次调度选定的同一个节点执行。新建 `system` 执行和跨节点团队调用已关闭；历史 `system` 记录仍可按 ID 查询、取消或中断，但不能恢复。管理员需要维护某个节点时，必须显式指定该节点调用 maintenance 入口。

维护仅响应用户明确提交的状态查询、配置修改、任务控制或自然语言维护指令。注册、心跳、离线和错误本身不会触发自动修复。

大脑能力可绑定 agent、team、DAG 或 TODO 模板。预览只产生路由结果；直接调度调用统一执行入口。客户端提供 `request_id` 时不能再提供自定义执行 ID；同一规范请求重复提交复用原执行，不因节点离线改变归属，也不会再次启动。已有执行缺少或不匹配 receipt 时返回冲突。进程在写入 Pending 索引前崩溃可能重复调用规划模型，但不允许产生第二条执行。

## API

所有管理 HTTP、SSE 与 Node WebSocket 使用 `Authorization: Bearer <token>`。token 区分大小写；缺失或错误凭据返回 401。合法请求不依赖时间同步、nonce 或签名重放缓存，401 也不会自动重放修改请求。`GET /api/time` 仅保留为普通兼容端点。

| 操作 | API |
| --- | --- |
| 注册通道、节点列表 | `GET /api/nodes/channel`（WebSocket）、`GET /api/nodes` |
| 创建、游标列表、明细 | `POST /api/executions`、`GET /api/executions?limit=&cursor_created_at=&cursor_id=`、`GET /api/executions/:id` |
| 控制、事件与大字段 | `POST /api/executions/:id/commands`、`GET /api/executions/:id/events`、`GET /api/executions/:id/messages`、`GET /api/executions/:id/detail-field` |
| drain 与就绪 | `GET /api/ready`、`GET/POST/DELETE /api/admin/drain` |
| 节点并发与排队配置 | `GET/PUT /api/nodes/:id/scheduling`，请求 `{ "max_runs": 4, "queue_order": "fifo", "workdir": "/abs/workspace" }`；host 节点读接口返回 `workdir_supported:false` 且拒绝 workdir |
| Agent 资源读取 / 保存 | `GET/PUT /api/agents/:name/resources/:cat`（`cat=prompts/skills/tools/memory`） |
| Agent 资源恢复 | `POST /api/agents/:name/resources/:cat/restore` |
| Harness 配置 | `GET /api/harnesses`、`PUT /api/harnesses/codex` |
| 显式节点维护 | `POST /api/nodes/:id/maintenance` |
| 团队定义 | `GET/POST /api/teams` |
| 能力绑定、直接调度 | `PUT /api/brain/capabilities/:id/target`、`POST /api/brain/dispatch` |
| 定时调度定义 CRUD、手动触发与触发历史 | `GET/POST /api/schedules`、`PUT/PATCH/DELETE /api/schedules/:id`、`POST /api/schedules/:id/run`、`GET /api/schedules/:id/runs?limit=` |

资源读取返回 `baseline: {resource, version, revision}`、`versions`、递归 `files: [{path, content_b64, mode}]` 和 `read_only`。保存提交读取时的 `baseline`、新增或修改的 `files` 和删除路径 `removed`；未提交文件保留。恢复提交 `baseline` 与历史 `version`，响应与读取相同。文件模式仅接受 `0o000–0o777`，合并后的资源上限为 1.5 MiB / 4096 文件。旧共享资源池 API 保留；PUT 以 URL 名称为准，可省略 body 名称，名称不匹配报错，专属资源必须经所属 Agent 接口修改。

创建示例：

```json
{"id":"agent-client-request-1","kind":"agent","target":"act","input":{"prompt":"检查当前仓库"},"node_id":null}
```

DAG 页和执行详情先展示节点结果快照，运行中只折叠快照之后的状态事件，不逐条回放历史来绘制画布。`GET /api/dag/runs/:id/progress` 和执行详情的 `dag_steps` 返回 `head_seq`、步骤状态及 `running` 计数；新一轮步骤开始可覆盖旧回执，断线后重新同步快照。点击步骤打开右侧占视口 75% 的「实时日志」抽屉，共用步骤切换、全部步骤、搜索、自动滚动和历史分页；历史记录整批展示，关闭抽屉即结束日志请求。日志展示 Agent 输出、思考与工具事件以及 Wasm stdout/stderr。日志沿 Node WebSocket 与浏览器 SSE 增量传输，断线从已接收的 seq 续传；服务端明确发送流结束标记，网络断开不会显示为正常结束。事件支持 seq 回放；超大事件、消息和详情字段由 64 KiB chunk 及游标分段读取。DAG 产物通过 Bearer 保护的流式下载端点传输，256 MiB 验收不会在浏览器或 Server 聚合完整文件。原会话、DAG、TODO、Team 和项目页面 API 均由 Server 依据五字段索引转发到归属节点。

## 验证边界

自动测试覆盖真实 Server ↔ Node WebSocket、断线后继续执行、同 ID 幂等、项目节点绑定、普通团队和 DAG 单节点闭环、拒绝新 System 执行、资源快照和检查点恢复。`scripts/acceptance/platform.js` 及 `scripts/acceptance/t12_ui_verify.js` 使用真实二进制、两个临时节点、回环模型服务及 Chromium 验证打包后的页面；回环模型只证明控制与执行闭环，不代表目标模型凭据已经验收。

真实 NFS 需要验证只读写入拒绝、版本资源快照，以及卸载后旧执行继续和新执行拒绝。依赖宿主权限的 NFS/runc 用例保留 manual 标记，验收记录必须对应当前执行后端和实际节点环境。

DAG 的非 Agent 步骤为 WebAssembly WASI 命令模块。默认 `sandbox: in_process` 使用内嵌 wasmtime，通过 epoch deadline 处理取消和超时，无需单独安装 wasmtime CLI。`sandbox: runc` 需要节点上的 runc 和 `<workflow_root>/rootfs` 中可运行的静态 wasmtime 目录；缺失时报错，不回退到内嵌模式。每个 bundle 复制独立运行时目录并在重试中复用，rootfs 只读挂载，run 目录挂至 `/workspace/context`。模块读取 `OPENCODER_STEP_CONTEXT` 指向的 `context.json`，可写 `output.json` 返回结构化结果；不存在 `internal-python-step` 或 RustPython 执行入口。wasm 模块经 Server 模块池发布（`/api/dag/wasm` + 第二路 NFS 只读导出），节点配置 `dag.wasm_dir` 后受理时冻结到 `_modules/`，spec 中 `tool@v3.wasm` 形态可显式固定版本。详见 [DAG 运行时](../agents/dag-runtime/index.md) 与 [dag-wasm 模块](../agents/dag-wasm/index.md)。

动态节点支持按派发输入或上游结构化输出批量展开 Agent/Wasm 实例，逐实例隔离 how、argv、状态和日志；四个并发名额在整个 run 内共享。定义示例、恢复规则和实例 API 见 [Dynamic DAG Step](dag-dynamic.md)。

## 发布与回滚

平滑发布使用固定 Nginx 入口、双版本 Server、稳定 Agent Host 和独立 Runtime。完整配置、首次迁移、发布、回滚、备份及验收见 [平滑发布](smooth-release.md)。

兼容发布只激活候选版本并 reload 入口；旧任务原地完成，不调用全局 drain。原有两进程部署需要先执行一次安全迁移窗口，保留 Node ID、凭证和历史数据。跨版本 Host 共用并发上限和 FIFO，不支持节点级 workdir（配置返回 400）；旧 Runtime 在任务、工具、写入及 Brain 回执全部结束后休眠，历史访问可唤醒。

版本回滚切换新流量与新任务归属，不恢复旧数据库。NFS 资源服务及其升级使用独立维护流程。首次迁移后不要再用原 Agent/Server unit 重启命令代替发布工具。

手动依赖验收入口：

```bash
cargo test -p opencoder-worker --test nfs_mount -- --ignored --nocapture
DAG_TEST_ROOTFS=/path/to/rootfs cargo test -p opencoder-dag-runtime sandbox::runc::tests:: -- --ignored --nocapture
PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/platform.js
PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/node_drain.js
FIXTURE_BYTES=268435456 PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/artifact_stream.js
```
