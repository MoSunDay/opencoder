# Agent 调度平台

`opencoder` 保留 CLI/TUI；平台由两个独立进程组成：`opencoder-server` 负责 Web、全局定义、大脑和调度，`opencoder-agent` 负责节点上的 agent loop。Server 不创建本地会话、不执行团队或工作流，也不链接 Python VM/runc。

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

Node 在返回接受前同步持久化任务、资源和 Harness 配置快照。节点页可设置最大顶层并发数（1–65535）和 FIFO/LIFO；`scheduling.json` 保存配置，优先于重启时的启动默认值。超额任务以 pending 等待，释放容量后按配置顺序启动；降低上限不打断正在运行的任务。创建请求携带稳定 ID：同一 ID、相同输入重复提交不重复执行；不同输入返回 409。网络超时后归属仍保留，客户端必须用原 ID 重试，Server 不猜测任务是否接受而改派。

断线只影响传输，已接受的本地工作继续。Node 重启将原先运行中的执行标记 `interrupted`，由用户显式在原节点恢复；已接受但尚未开始的持久化 pending 队列继续等待，冻结节点在复开后才继续调度。DAG 恢复跳过已成功写入检查点的步骤。项目执行 ID 为 `project-<todo-id>`，Plan、Act 和新一轮 Plan 保持相同节点。节点存储出现持久化错误时上报不可调度，须处理存储故障后重启节点。项目页面与全部执行详情中的 Plan 均由 Server 解析当前草稿；Node 在忙碌及资源预检通过后才保存该轮快照，拒绝请求不修改之前的运行记录；容量不足会保存并排队，等待期间 Project 运行不会被失联清理误判。

## NFS 资源共享

Web「Agent 配置」顶部包含 Agent 列表、Agent Harness、Harness 管理、NFS 配置。主列表展示每个 Agent 的资源引用、当前版本、目录和内容；Codex 的二进制、模型、推理、权限参数和 env 在 Harness 管理中统一保存，正在运行、排队及续聊的会话保持已接受的参数。共享内容限于 agent 定义及 prompts/skills/tools/memory，不共享 runtime DB、对话、项目运行记录或 DAG 产物。

使用页面提供的挂载命令，并保留 `ro`：

```bash
mount -t nfs -o ro,vers=3,tcp,port=<port>,mountport=<port>,nolock,soft,retrans=1,timeo=50 server:/ /mnt/opencoder-agents
```

在 Node 的 `opencoder.json` 中设置 `agent.agents_dir` 为 `/mnt/opencoder-agents`。显式配置该路径时 Node 校验 Linux 挂载表，要求可读的只读 NFS；未挂载、可写挂载或本地目录都返回资源错误，不静默使用本地资源替代。

每次新执行复制当前版本到节点资源快照，包含实际文件。资源后续发布、回滚或移除不会改变已接受的执行。缺失引用、不可读资源和版本内符号链接在接受前报错。已有执行的继续或恢复使用已固定快照，NFS 断开不阻止这些操作；新执行需要共享目录可用。仅使用内置 agent 时可以不配置共享目录。

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
| 节点并发与排队配置 | `PUT /api/nodes/:id/scheduling`，请求 `{ "max_runs": 4, "queue_order": "fifo" }` |
| Harness 配置 | `GET /api/harnesses`、`PUT /api/harnesses/codex` |
| 显式节点维护 | `POST /api/nodes/:id/maintenance` |
| 团队定义 | `GET/POST /api/teams` |
| 能力绑定、直接调度 | `PUT /api/brain/capabilities/:id/target`、`POST /api/brain/dispatch` |

创建示例：

```json
{"id":"agent-client-request-1","kind":"agent","target":"act","input":{"prompt":"检查当前仓库"},"node_id":null}
```

事件支持 seq 回放；超大事件、消息和详情字段由 64 KiB chunk 及游标分段读取。DAG 产物通过 Bearer 保护的流式下载端点传输，256 MiB 验收不会在浏览器或 Server 聚合完整文件。原会话、DAG、TODO、Team 和项目页面 API 均由 Server 依据五字段索引转发到归属节点。

## 验证边界

自动测试覆盖真实 Server ↔ Node WebSocket、断线后继续执行、同 ID 幂等、项目节点绑定、普通团队和 DAG 单节点闭环、拒绝新 System 执行、资源快照和检查点恢复。`scripts/acceptance/platform.js` 及 `scripts/acceptance/t12_ui_verify.js` 使用真实二进制、两个临时节点、回环模型服务及 Chromium 验证打包后的页面；回环模型只证明控制与执行闭环，不代表目标模型凭据已经验收。

真实 NFS 需要验证只读写入拒绝、版本资源快照，以及卸载后旧执行继续和新执行拒绝。依赖宿主权限的 NFS/runc 用例保留 manual 标记，验收记录必须对应当前执行后端和实际节点环境。

DAG 的非 Agent 步骤为 WebAssembly WASI 命令模块。默认 `sandbox: in_process` 使用内嵌 wasmtime，通过 epoch deadline 处理取消和超时，无需单独安装 wasmtime CLI。`sandbox: runc` 需要节点上的 runc 和 `<workflow_root>/rootfs` 中可运行的静态 wasmtime 目录；缺失时报错，不回退到内嵌模式。每个 bundle 复制独立运行时目录并在重试中复用，rootfs 只读挂载，run 目录挂至 `/workspace/context`。模块读取 `OPENCODER_STEP_CONTEXT` 指向的 `context.json`，可写 `output.json` 返回结构化结果；不存在 `internal-python-step` 或 RustPython 执行入口。详见 [DAG 运行时](../agents/dag-runtime/index.md)。

## 发布与回滚

Fleet 协议 v6 要求 Server 与 Node 成套更新，旧版本连接会被拒绝。本轮不增加数据库表或环境变量；Harness 值复用私有定义库，队列和节点调度配置保存在 Node 数据目录。

1. 在干净提交上运行 `scripts/platform/release/build.sh --output <新目录>`。脚本先验证 SPA 无漂移，再一次构建 `opencoder`、`opencoder-cli`、`opencoder-server`、`opencoder-agent`；四者的 `--build-info` 必须具有相同 commit、protocol 和 SPA digest，`manifest.json` 与 `SHA256SUMS` 绑定全部二进制。真实 dirty 工作树会被拒绝。
2. 调用 `POST /api/admin/drain` 先持久冻结 Server 与当前在线 Node。最多观察 10 分钟让长任务自然完成；仍未完成的任务（包括尚未启动的 pending）逐条显式 interrupt，并等待 `GET /api/admin/drain` 返回 `control_drained=true`。取消后的执行不能恢复；interrupt 后只能在原 Node 显式恢复。30 秒只用于 interrupt/进程树清理，不能用作长任务的自然 drain 时限。
3. 正常停止 Agent，再停止 Server。Agent 的本地 Frozen 状态跨重启保留；Server 也以 Frozen 重启。使用 `scripts/platform/backup.sh --server-data /var/lib/opencoder-server --node worker-a=/data00 --output <新备份目录>` 制作新备份；工具要求 Node 已停止、持有独占锁、所有执行收敛并对 SQLite 做一致性备份，不覆盖任何原目录。跨主机发布必须先按目标 inventory 在各主机停服并把这些本地目录提供给受控备份步骤，不能把本机演练当成跨机备份证据。
4. 使用 `scripts/install.sh --bundle <bundle> --dest-dir /usr/local/bin --backup` 原子安装 manifest 声明的同一代二进制。先启动 Server，再启动 Agent；检查 manifest/build-info、目标 Node 清单、节点 ID、版本、资源挂载和 Ready。所有发布目标都到齐后调用 `DELETE /api/admin/drain`，它只复开当前在线且健康的节点；离线节点会明确列为未处理，不会伪装完成。
5. 用普通 Agent、DAG、Team、TODO/Project 和大脑稳定 `request_id` 各走一条真实链路，并验证列表五字段、所属 Node 明细、控制动作、大字段分段和产物下载。在真实拓扑连续观察 2 小时：Server/Agent 无意外重启，目标 Node 全部 Ready，无新增认证、协议、存储或进程清理错误，无超过 60 秒仍未确认的 Pending，也无超过 30 秒仍未收敛的 interrupt。
6. 失败时立即重新冻结，不删除索引或执行数据。二进制回滚用 `scripts/platform/rollback.sh --bundle <上一代bundle> --dest-dir /usr/local/bin` 成套切换全部二进制；需要回滚数据时先用 `scripts/platform/restore.sh --backup <备份> --output <新的隔离目录>` 验证并恢复到新目录，按实际服务身份设置新恢复目录的 owner 和读写权限，再让 Server/各 Node 的 `--data-dir` 明确指向对应恢复目录。不得把旧二进制直接指向未经配套验证的新协议数据，也不得覆盖原数据目录。

首次平台部署保留旧 CLI/daemon 目录；回退旧模式时仍使用其原目录，不能让旧版本读取新平台库。仓库内验收没有执行生产部署，也没有修改生产数据或凭据。实际主机地址、目标 inventory、内网证书路径、NFS 挂载和模型凭据引用是上线时必须填写的外部参数。

手动依赖验收入口：

```bash
cargo test -p opencoder-worker --test nfs_mount -- --ignored --nocapture
DAG_TEST_ROOTFS=/path/to/rootfs cargo test -p opencoder-dag-runtime sandbox::runc::tests:: -- --ignored --nocapture
PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/platform.js
PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/node_drain.js
FIXTURE_BYTES=268435456 PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/artifact_stream.js
```
