Commit: 347a6bdfee28570f5c5cf9e2e1891d563cdf1bb7

# 平滑发布

固定 Nginx 入口连接当前 Server。稳定 Node ID 属于 Agent Host；每个版本使用独立 Runtime 进程、systemd unit、目录和 `runtime.db`。新版本激活只改变全新执行的归属。已有执行、续聊、TODO 重跑、Brain 子执行、历史和产物继续访问原 Runtime。

## 首次迁移

首次迁移需要一次维护窗口。先让当前节点的执行、队列和工具自然结束；迁移工具复核空闲后冻结接入，停止旧 Agent/Server，建立一致性备份，再启用独立资源服务、Host、Runtime 和入口。保留 Node ID、原执行目录、Server 库和凭证。不通过恢复旧库回滚版本。

首次迁移前，在现有 Server `opencoder.json` 中加入 `deployment`，按当前机器填写路径、服务账号、监听地址和原并发上限：

```json
{
  "deployment": {
    "state_dir": "/data00/opencoder/platform",
    "server_workdir": "/etc/opencoder/server",
    "server_data": "/data00/opencoder/server",
    "server_user": "opencoder-server",
    "agent_workdir": "/srv/opencoder",
    "legacy_agent_data": "/var/lib/opencoder-node",
    "token_file": "/etc/opencoder/server.token",
    "node_name": "worker-a",
    "max_runs": 20,
    "public_url": "http://127.0.0.1:18081",
    "listen": "0.0.0.0:18081"
  }
}
```

默认 Host 本地入口为 `127.0.0.1:18082`，资源管理服务为 `127.0.0.1:18084`，版本实例端口从 `3000` 分配。版本端口应位于主机临时客户端端口范围之外，避免预热或休眠期间被客户端连接占用。SQLite 和锁文件必须放在本机本地磁盘。保留现有服务环境配置及原凭证文件，配置和发布清单不包含凭证明文。

`server_data` 传给 Server 的 `--data-dir`；`state_dir` 包含 `host/`、`resources/`、`runtimes/<版本>/`、`releases/` 和 `backups/`。发布工具把每个 Runtime 目录传给 Agent 的 `--data-dir`，DAG 过程文件位于该目录的 `dag/runs/`。两个配置项都应指向已挂载的数据磁盘，并提前设置服务账号权限。已有部署不能仅修改路径后启动空目录：应迁移完整数据、保留节点身份及锁文件，并保持已接受执行记录中的旧路径可访问。

```bash
# 没有 Nginx 时执行一次；不会启动服务监听。
scripts/platform/ingress/install.sh

scripts/platform/release/build.sh --output /srv/releases/opencoder-first
scripts/platform/deploy.sh --bundle /srv/releases/opencoder-first --migration-receipt
scripts/platform/deploy.sh --bundle /srv/releases/opencoder-first --migrate --wait-seconds 600
```

迁移收据列出 Node ID、旧服务、数据目录和备份位置。等待预算用尽会报错，不能据此中断任务。已停止旧服务后的中断可用同一命令继续；`migration_stage` 和已完成备份决定恢复位置。资源服务升级、整机重启和破坏性存储迁移属于另行安排的维护操作。

## 兼容版本发布与回滚

```bash
scripts/platform/release/build.sh --output /srv/releases/opencoder-next
scripts/platform/deploy.sh --signal --bundle /srv/releases/opencoder-next --wait-seconds 300
scripts/platform/deploy.sh --status
scripts/platform/deploy.sh --signal --rollback --wait-seconds 300
```

Linux 发布包包含四个应用二进制及 `dag-runner`、`agent-step-runner`，附带摘要、`release_id`、协议及数据兼容范围。发布工具核验所有仍运行的版本，取得互斥锁，检查磁盘和可用内存，再依次执行：

1. **校验、预热：**启动候选 Runtime，执行确定 ID 的 Linux 二进制探针；启动候选 Host 和 Server，连接并同步完整索引，核验资源可读且导出只读。
2. **就绪、切换：**先落盘切换意图，再激活 Runtime/Host，graceful reload Nginx。Host 通道按递增交接编号切换；旧 Host 等待各 Server 的完整索引确认及入口确认。
3. **验证、完成：**公共入口核对版本并实际执行探针，更新命令行二进制入口，记录完成。旧 Server 停止监听并等现有响应结束；旧 Runtime 单独回收。

候选探针使用统一任务容量，不能抢占旧任务。候选所需资源不足或探针在等待预算内未完成时，预热报错，当前版本继续服务。同一次发布可以中断续跑；请求与探针保持原 ID。回滚或重新激活会持久化新的探针批次，确保通过当前入口真正执行新任务。

切换前失败保留当前服务；切换后验证失败回到上一兼容版本。回滚或再发布保留版本时，都启动额外 Server/Host 实例，不等待仍在处理请求的旧实例退出；同一操作中断续跑保持实例和探针身份。已被新版本接收的任务留在新 Runtime。回滚不恢复数据库，不撤销已发生的工具副作用。不兼容版本直接拒绝平滑发布。

## 不兼容版本的维护发布

当前发布数据契约为 4，项目 schema v33 的 TODO 执行关联只保存能力与执行引用，移除结论和同步状态缓存；原生 DAG 与独立 Ontology 存储继续使用。旧 Server 不能打开升级后的项目库，格式 1–3 升级到 4 必须走维护发布，不能让新旧 Server 同时写入。普通滚动发布和暂存会在启动候选进程前拒绝从旧格式直接升级。已完成首次迁移的单机安装可用 `--migration-receipt` 查看维护范围，使用 `--migrate` 执行；信号发布可使用维护模式：

```bash
scripts/platform/deploy.sh --maintenance --stage --bundle /srv/releases/opencoder-native
scripts/platform/deploy.sh --maintenance --signal --bundle /srv/releases/opencoder-native --wait-seconds 600
# 也可由独立 root 作业直接执行，崩溃后用同一命令继续。
scripts/platform/deploy.sh --maintenance --bundle /srv/releases/opencoder-native --wait-seconds 600
scripts/platform/deploy.sh --rollback --wait-seconds 600
```

迁移新旧配置分开放置：可在 deployment 配置中指定绝对路径 `agent_config`、`server_config`，指向本次维护的候选配置 JSON；DAG 段是完整的新声明，其余服务设置在原配置基础上覆盖。预检固定摘要和私有快照，停止旧服务并完成备份后才安装，续跑读取同一快照。停服前必须已有完整的候选配置：Agent 的 `dag.rootfs_dir`、`dag.binary_dir`、`dag.workspace_dir`、`agent.agents_dir` 都是绝对路径；Server 显式配置三个资源目录，启用各自独立端口的只读 NFS。既有 Agent 客户端目录必须已经挂载为可读的只读 NFS。原生二进制目录和工作区目录在预检时保存挂载计划；停止旧服务并完成备份后，先升级资源服务，再建立这两个只读挂载并核验实际资源。镜像中的本版本两个 runner 必须能够启动。本维护流程管理 Linux/libsql 本机服务；缺失配置、旧 DAG 配置和在线的外部节点会在停服前拒绝。外部节点须先冻结、结束或经授权取消任务并单独停服，保存服务和进程恢复信息；不删除离线节点注册及其执行历史。排空依据本机节点的冻结回复、实际运行数、所属进程数和每个保留 Runtime 的最终库存，不将历史空闲会话等同于正在运行的任务。等待期间外部节点重新接入会继续阻止停服。

容量预检按实际目标文件系统计算新镜像、共享数据的停服备份、服务与配置副本、资源数据及两个发布包副本，并预留数据库迁移和安全余量。镜像冻结后、关闭入口前再次检查；已有副本和中断留下的文件占用当前可用空间。容量不足时保留旧服务和入口，不开始停服。

Ontology 正文使用第四个只读导出，默认关闭。首次启用时在候选 Server 配置中显式设置 `ontology.nfs.enabled=true`，可配置 `ontology.files_dir` 和独立端口（默认 `127.0.0.1:2052`）；路径与三个既有资源根分开，不能包含 Server 数据库。资源服务须随维护升级，再验证实际正文读取与写入拒绝。配置说明见 [Ontology](../features/ontology/index.md)。

关闭、恢复及重新开放入口时，都等待旧 Nginx worker 关闭监听 socket 后再继续；已有节点长连接可以继续存活。仅收到 reload 成功不能证明新请求已切换。

维护发布先将公共入口关闭为 503（保留节点通道），冻结新任务并等待已接纳任务结束，再停止所有保留版本的 Server、Host、Runtime 和资源服务。停服前确认 Runtime 没有运行进程或保留容量，并保存它的最终库存信息。停服后保存独立备份目录，包含 Server、资源服务及 Host 的共享数据库和文件、旧包校验、配置、systemd 服务、挂载信息、控制器模板和命令行入口。历史 Runtime 和旧节点的执行目录原地保留，升级与回滚都不改写它们，因此不再次复制镜像、会话和产物；随后使用现有休眠记录保留旧 Runtime 的索引，让新 Host 可以启动。资源服务二进制和 unit 随候选一起升级；每个 Runtime 使用独立镜像。候选 Server 执行事务迁移后，通过私有 DAG 探针、项目读取和资源验证，才重新开放 admission 和公共入口。

`release-state.json` 的 `maintenance` 保存候选、备份地址、阶段和阶段时间。中断后续跑使用同一备份、配置快照和候选；完整备份不被重试覆盖。启动候选 Server 的迁移意图先落盘，但旧备份恢复的边界是重新开放写入的持久化意图，不能用迁移是否已经开始替代这一边界。

重新开放写入前，`--rollback` 可以停止候选，恢复维护前的项目表、索引和 schema 版本，再恢复资源服务、Host 状态、配置、控制器和入口；已经执行迁移也可以恢复。Server 和资源库的认证表与数据库 inode 保持原值，资源控制文件恢复不覆盖数据库；Ontology 库与正文一起恢复。不一致或备份校验失败时停止恢复。旧 Server 通过项目读写检查和 admission 复开验证后才恢复公共入口。

记录 `writes_open=true` 后拒绝旧备份恢复，避免覆盖新写入。若公共验证失败，可用新的不可变发布 ID，通过 `--maintenance --stage` 或 `--maintenance --signal` 发布与当前协议及数据格式一致的修复版本；修复沿用普通滚动流程，保留最初的封存备份和配置快照。修复中断后须重试已记录的修复候选。维护完成后的滚动发布与回滚仅接受数据契约 4、schema v33 的兼容版本，已停止的旧格式版本不再参与。

Server 的源工作区必须预先存在，并允许实际资源服务账号读取和进入；预检及资源服务升级都会检查这项权限。维护工具不创建源目录、不修改源目录所有者或权限，也不移动源路径。二进制池、客户端挂载、服务配置文件和会备份恢复的状态目录不能与源工作区重合，也不能互相包含。只有应用管理的二进制池和服务状态目录会自动准备权限。

## Server 信号入口

首次启用信号时，先用 `deploy.sh --bundle ...` 普通平滑发布安装支持信号的 Server/Host。之后 `/api/admin/release` 返回 `signal_protocol: 1`；工具会核对当前实例和能力再发信号。旧二进制的 USR1/USR2 默认会终止进程，不能绕过能力检查。

- **USR2：**发布已暂存的包；`deploy.sh --stage --bundle ...` 校验并保存不可变候选，`--signal --bundle ...` 自动完成暂存、发信号和等待回执。
- **USR1：**回到上一兼容版本；重复回滚保持同一目标，已接收任务留在各自 Runtime。
- Server 经认证的本机 Host 启动独立 systemd 作业；当前 Server/Host 退役不会终止作业。控制器脚本固定到 `state_dir/controllers/<digest>`，不依赖正在编辑的仓库文件。
- 若手动发信号，先从发布记录的 `releases[current].server_unit` 取得当前 unit 并核验能力，再执行 `systemctl kill --kill-who=main --signal=SIGUSR2 <unit>`；回滚使用 SIGUSR1。发布工具使用管理员的 systemd 管理权限。
- 作业启动、完成和失败保存到 `state_dir/signal-receipts`；`deploy.sh --status` 的 `signals` 返回候选和回执。等待超时后先检查作业与回执，不能将旧的成功回执当成本次结果，也不能通过重启服务进程处理超时。

完整发布工作流见 [opencoder-release skill](../skills/opencoder-release/SKILL.md)。

## 归属、容量与恢复

- `control.db` 分开保存请求指纹、冻结 assignment、派发阶段和五字段索引。相同 ID/相同请求重放同一回执；变更内容返回 409。已明确拒绝的派发也保留回执。Server 恢复时自动重试仍待确认的持久派发。
- Host 的 `host.db` 保存 Runtime 注册、不可变执行归属和全机容量队列。所有版本合计使用同一个并发上限和 FIFO；降低上限不会停止当前执行。多版本 Host 不支持 LIFO。
- 跨进程长操作使用本机文件锁；SQLite 的写事务仅覆盖短提交，不覆盖模型或网络调用。锁文件不能被清理；进程退出由内核释放锁。
- Runtime 写入 `host-binding.json` 后使用共享容量账本，在启动执行前取得槽位，完成持久化后释放。心跳失联、Server 退出、发布或回滚均不释放运行槽位。
- Runtime 的 `global-skills` 固定全局用户技能及本版本内嵌技能；Host 启动不会改写共享技能目录。后续发布继承当前 Runtime 已配置的 OCI 镜像并保留私有副本；镜像为必需项，预热必须通过真实 runc 探针，且镜像中的两个运行器必须来自同一版本包。
- Runtime 意外退出遗留运行槽位时，启动明确拒绝未解决的状态；需在独立维护流程中核实原进程和容器已退出，不能直接按超时回收或自动重跑。
- 节点冻结或重新开放任务时保持休眠 Runtime 停止；实际访问并唤醒时，在放行请求前同步当前任务开关。休眠要求无运行、排队、工具进程、执行 future、持久化错误和待确认 Brain outbox。Host 保留最终索引、版本包和数据；原执行查询或续跑会通过独立 unit 唤醒 Runtime。回收与使用同一 Runtime 的 RPC 使用互斥/共享文件锁协调。

## 页面、事件与接口

节点页的「发布状态」展示当前版本、候选、阶段、失败原因和每个 Runtime 的排队/运行任务。旧版本可以打开执行详情；休眠与回收失败明确显示。

| 接口 | 用途 |
| --- | --- |
| `GET /api/admin/release` | 发布记录及 Host 的 Runtime/容量状态 |
| Host 本机 `POST /deployment-signal` | 认证并核验当前实例，启动固定发布/回滚作业 |
| `POST /api/admin/release/retire` | 仅退役当前 Server，不改变集群 admission |
| `GET /api/executions/:id/receipt` | 查询持久派发阶段和确定回执 |
| `GET/POST/DELETE /api/admin/drain` | 管理员显式冻结/复开，与发布退役分离 |

Server 退役时 SSE 发出 `reconnect` 和最后已发送的游标。页面自动重连、补齐后续事件；已有游标不会因版本切换被头部水位替换。普通响应没有发布强杀期限。

独立资源服务使用原 Server 账号、工作目录、导出目录和 NFS 端口。Server 的四组 NFS 管理接口转发到资源服务。挂载与 Runtime 不再使用 `PartOf=opencoder-server.service`；常规发布只 reload Nginx，保持 NFS 进程和挂载。Server 退休等待已接纳的 Ontology 写事务结束，避免客户端断开或版本切换取消保存。

Project 会话保持稳定的 `project-<todo id>` 归属，首次受理回执按 `run_id` 区分。相同 run ID 的重试返回原回执；首次明确拒绝后，可以使用新的 run ID 发起规划，旧归属保留。结果尚不明确的派发不能被新请求替换。可通过 `/api/executions/<run_id>/receipt` 查询首次受理结果。

旧 Runtime 启动若报告未结束的运行名额，不能按时间自动释放。经授权取消该执行后，先停止对应 Runtime，并确认其 systemd 单元、内核进程和节点锁都已释放。可执行 `python3 scripts/platform/rolling/maintenance/recovery/capacity.py --config <配置> --runtime <旧Runtime ID> --execution <执行ID> --ticket <名额ID>`：工具要求 Server 和本机节点已冻结，且没有其他未结束名额；这一条遗留名额造成的逻辑运行计数可以为 1。核对所属关系、保存不可覆盖的 Host 库及受理状态备份后，先在停机状态下冻结旧 Runtime，再只结束这一条名额。若 Host 的请求仍持有锁，在取得当前冻结且无任务进程的证明后，暂时停止准确的旧 Host 单元，完成恢复后启动同一版本。再启动原 Runtime，通过原接口完成取消；进程尚在、身份不符或备份损坏都会拒绝操作，重试沿用原备份。

## 备份与验收

```bash
scripts/platform/deploy.sh --backup /srv/backups/opencoder-online-001
```

在线备份逐库使用 SQLite backup API 并检查完整性，明确标记为独立数据库备份，不能当成跨库同一时刻快照。首次维护窗口另行保存停服后的完整一致性备份，包括嵌套数据库。中断的备份保留在独立 staging 目录；重试不覆盖已有备份。

旧 OCI 目录里的字符设备、块设备和管道只复制类型、设备编号及权限，不读取其内容；备份校验和恢复同样核对这些元信息。容量计算按元信息所需空间计入，不能将 `/dev/full` 等设备复制成字节文件。无法归档的 socket 在关闭入口前的容量预检中拒绝。

维护预检按实际复制内容汇总每个文件系统的空间：新冻结镜像、共享数据备份、控制配置、暂存和安装发布包，以及数据库迁移空间。历史执行目录不重复占用备份容量；预检收据明确列出备份根与原地保留的执行根。关闭入口前再次核验；已有副本和中断的暂存目录继续占用实际可用空间。

迁移完成后、开放写入前可以恢复旧项目表、索引和 schema，并保留认证数据及数据库文件。记录开放写入意图后禁止恢复旧备份。此后公共验收失败，可用 `--stage --maintenance --bundle <新的兼容修复包>` 暂存修复版本，再通过 Server 信号启动；修复沿用兼容发布流程，保留原备份和恢复记录，进程中断后可继续同一修复版本。

已有 Ontology 数据库的正文根不能通过改配置重新绑定；即使库为空也保留首次绑定。维护预检在关闭受理和停止服务前拒绝目录变化，恢复时须保留原路径并同时恢复数据库与正文。

Ontology 备份先取得 ontology.db 快照，再复制数据库记录的实际正文根，包括配置在 Server 数据目录之外的正文。每个有效引用核验 SHA256 与字节数；维护恢复和数据归档同时处理数据库和正文，不能只恢复数据库或只复制默认目录。

发布验收必须包含跨切换 TODO 依赖链、持续 DAG 工具任务和持续新任务流，并核对原进程、执行归属、FIFO、容量、日志游标、历史与产物。模拟故障覆盖发布工具/Server 中断、重复请求、回滚和三版并存。最终切换后观察 15 分钟。仓库测试与真实运行证据分别记录，不能用启动成功或模拟测试代替真实验收。

平滑切换允许最多 30 秒的受理延迟、连续受理间隔和调度间隔，不设 1 秒门槛。隔离进程演练检查最大值；真实入口验收另行记录 P95，并要求公共探测无失败、连续成功探测间隔不超过 30 秒。30 秒边界通过，超过即失败。

隔离进程演练入口为 `scripts/acceptance/smooth_release/main.py --bin-dir <已构建二进制目录> --nginx <nginx路径> --rootfs <已准备的原生镜像>`。它启动私有 Server/Host、独立 systemd Runtime、只读 NFS 和持续请求流，保留日志与数据库。所有 DAG 探针均验证真实 OCI 容器；工具为各 Runtime 准备私有镜像，保留镜像内部硬链接，副本不与源镜像共享可写 inode。`--data-parent` 可选择隔离测试存储；先用 `findmnt -T` 核对它与正式数据库、Runtime 写层所在的文件系统。使用内存文件系统的结果仅用于功能与竞态验证，不能充当生产磁盘的延迟或持久性验收。步骤结束后的长时间等待还须检查 OverlayFS 卸载和底层磁盘写回，任务完成必须等待实际清理结束。

隔离演练加入 `--observe-seconds 900` 完成切换、回滚后的 15 分钟观察：持续提交真实任务，并重复核验 Ontology 当前正文、历史版本、切面与只读挂载。未达到完整观察时长或任何检查失败，均不能生成通过回执。

首次迁移完成后，使用 `python3 scripts/acceptance/smooth_release/live.py --config <现有配置> --bundle <下一兼容版本包>` 做真实模型验收。该命令创建专用 TODO 依赖链和长原生二进制任务，运行正式发布命令，核对原 Runtime/Shell 进程、新任务归属和 SSE 游标，然后持续提交探针观察至少 900 秒。验收失败也只释放自身等待信号，保留执行与证据，不删除数据库或取消任务。应提前构建好下一版本包。

Runtime 使用私有挂载时，宿主看到的 `workspace` 可能只是空挂载点。当前 `release_native_gate` 文件辅助函数不跨挂载空间；独立验收作业须按任务所属 Runtime，通过 `nsenter --target <Runtime PID> --mount --root --wd=/ -- ...` 在正确视图中调用它。仅切换 `--mount` 仍可能沿用宿主的根目录视图。操作只限本次验收创建的等待标记，失败日志与复验回执均须保留。

加入 `--signal` 使用 Server 信号发布；两版均支持信号时，再加入 `--signal-roundtrip`，在新旧长任务仍运行时执行发布、回滚、再发布。验收脚本应由独立 systemd 作业运行，避免终端退出中断观察。独立信号失败演练入口为 `scripts/acceptance/signal_release/main.py --bin-dir <已构建二进制目录> --nginx <nginx路径> --rootfs <配套原生镜像>`，验证真实 USR1/USR2、重复信号、失败回执和 Server 继续服务。
