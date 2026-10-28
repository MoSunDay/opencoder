Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

Windows 节点只声明并接受 `ExecutionKind::Operator`；其他执行种类在准入时拒绝，不装配 DAG/runc/NFS。依赖与实现通过平台模块隔开；原生验收入口见 [windows_operator.rs](../../crates/worker/tests/windows_operator.rs)，用户边界见 [Windows](../../features/windows/index.md)。

## 索引
- [brain/v4/api.rs](../../crates/worker/src/brain/v4/api.rs) — context 的 IO 版本须与冻结请求一致；旧运行不开放产物绑定提示，决策校验拒绝生成新协议。旧输入输出适配保留在 [output/legacy.rs](../../crates/worker/src/brain/output/legacy.rs)，新 Worker 可继续服务旧 Server 的请求。
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- 普通 Agent/Operator 共用宿主机后台会话路径，按 Server 固定的 Agent 身份、资源与 Harness 执行；DAG 容器仍由 [dag-runtime](../dag-runtime/index.md) 管理。
- [operations/query/session](../../crates/worker/src/operations/query/session/mod.rs) — 已持久接收但尚未初始化的会话由 journal 提供元信息、空消息和零游标；已有会话行但执行仍在排队或运行时也保持事件订阅。[事件分页](../../crates/worker/src/operations/query/mod.rs)在续聊等待名额时返回未结束。未知 ID 保持 404，初始化前终止返回实际原因。覆盖见 [conversations](../../crates/worker/tests/conversations/main.rs)。
- [workloads/agent_how.rs](../../crates/worker/src/workloads/agent_how.rs) — Agent 的 how 追加契约与结果投影。
- [workloads/agent.rs](../../crates/worker/src/workloads/agent.rs) — Agent/Operator 宿主会话首轮使用队列投递，保留用户输入原文；Operator 默认规则由 [session](../session/index.md) 按会话类型构建，Maintenance 首轮附加维护指令。托管 Codex 设置由 Server 冻结。
- [workloads/agent/initialization.rs](../../crates/worker/src/workloads/agent/initialization.rs) — 首轮投递前补齐 Harness、启动环境、展示策略及 Server 模型设置；已有 session 行也执行该步骤。恢复保留已启动的 Codex 线程和冻结设置，Operator HOME 最后覆盖输入与托管环境。
- `crates/worker/src/operations/` — 准入/launch/维护命令/查询（含 `query/instances/`、`artifacts.rs`、`operator_env.rs` Operator 隔离快照、`operator_config.rs` 节点级 Operator 配置平面）
- [operations/queue](../../crates/worker/src/operations/queue/mod.rs) — 接受/调度循环节拍自适应：存在 pending run 时 100 毫秒，空闲退避到 500 毫秒
- [operations/command.rs](../../crates/worker/src/operations/command.rs) 在同一次 journal guard 内保存取消意图和停止中的 TODO 重跑状态，然后立即发送取消信号，避免持久化后再次排在库存/调度读取之后；阶段计时仅记录执行 ID 和耗时，不记录输入或凭据。定向回归见 [stop_signal.rs](../../crates/worker/src/operations/admission/tests/stop_signal.rs)，包含排队锁竞争与零模型真实 runc 清理。
- [operations/create/preflight_agents.rs](../../crates/worker/src/operations/create/preflight_agents.rs) — 项目 Agent、Team、DAG 的 Agent 依赖提取纯函数；DAG 仅收集 Agent 步骤。
- `crates/worker/src/state.rs`、`src/layout.rs`、`src/journal/` — runtime.db、执行布局与原子落盘（layout 含 `<data>/operator/<id>/{home,workspace}` 预留）
- `crates/worker/src/runtime/`、`src/resources.rs` — Runtime 归属与资源快照；Runtime 启动完成遗留 runc 清理和 journal 恢复后，容量恢复还要证明当前进程属于受管 systemd Runtime 且同一 cgroup 没有其他进程，并确认本进程没有托管子进程，才逐票据释放遗留 `running` reservation；无法证明时 fail-closed。
- `crates/worker/src/brain/` — schema 7 里程碑投影、模型激活、回执及唤醒；`brain/v4/` 实现节点调度，`workdir.rs` 提供能力工作空间接缝
- `tests/` — 集成测试

## Operator 执行隔离

- [operations/fork.rs](../../crates/worker/src/operations/fork.rs)、[operator_env/fork.rs](../../crates/worker/src/operations/operator_env/fork.rs) — 派生会话按父执行类型生成 ID、资源目录及请求；隔离 Operator 复制父配置、冻结 skills 与私有 Codex 状态，创建空 workspace，重新固定 Harness HOME 与环境版本。普通 Agent 沿用节点工作目录。

Linux 身份层：`operator_env/linux_identity.rs`（含 `tiers.rs`/`sweep.rs`）为每条 Operator 执行供给专用 `ocop-` 系统账号——home/workspace 归属该账号、配置快照冻结 root 0440、准入 fail-closed（非 root或缺 shadow-utils 拒绝，`operator.linux_identity: off` 逃生阀）、fork 供给新账号、终态回收属主并删号、供给前补扫遗留账号；访问档位 `operator.access`：readwrite 附私有 TMPDIR，readonly 冻结目录 root 0555 并经 `opencoder-ro` 附加组授予共享只读资源组读。身份经 `HarnessRuntime.run_as` 持久化，Windows 路径保持环境隔离。

配置平面：`operations/operator_config.rs` 维护节点数据根下的 `<data>/operator-config/`（`config.json` + `mcp|cli|skills|ap|schedules` 五域文件 + `skills/` 技能包目录）。首个 Operator 执行准入时（`state.rs::configuration_for` → `operator_configuration`）从节点 live 视图 `bootstrap` 一次，以私有权限按 first-writer-wins 发布，此后冻结：TUI/CLI 对共享 workdir 配置的保存不再进入 Operator 执行；`freeze_skills` 把平面 `skills/` 包 + 内置 seed 写入执行 home（用户全局池永远不是来源）；`operations/launch.rs` 用 `core::skill::with_execution` 把 workload 包进执行技能根，[Web 后台会话](../web/index.md) 继续传递该目录。

`operations/operator_env.rs` 为 `ExecutionKind::Operator` 提供按执行的 HOME/WORKSPACE 隔离：`materialize()` 在准入通过后把冻结配置快照（明文含 provider api key）以私有权限写入 `<data_dir>/operator/<id>/home/.opencoder/config.json`，`resolve()` 核验快照与 workspace。`env_pairs()` 产出 HOME 覆盖对，Windows 同时隔离 USERPROFILE、APPDATA、LOCALAPPDATA；fresh 会话在输入 envs 之后注入，随后经 harness envs 持久化，resume 由 `resume.rs` 重建 env_passthrough。`operations/create.rs` 准入接受 Operator 的显式 Harness 选择，Codex 预检合并托管设置与输入 envs；`workloads/agent.rs` 固定所选 Harness 和环境。配置加载走 core `Config::load_with_home`，web drain 栈经 `AppState.config_home` 穿参，执行目录由 `brain/workdir.rs` `session_dirs()` 裁定。Maintenance/Agent/Brain 不受影响。

会话泳道：Operator 执行的 Primary Session 创建时打 `kind='operator'`（其他 kind 同理，见 store 索引），默认清单泳道排除 operator 行；`service.rs::indexes()` 按 `row.kind` 精确解析已打标行，存量 NULL 行保留 id 前缀/标题回退。Agent 和 Operator 空闲终态均从持久化会话取最后一条非空 assistant 文本，以有界 `output_text` 写入执行结果；Maintenance 保留会话指针结果。

## 接缝
- [brain/v4/budget.rs](../../crates/worker/src/brain/v4/budget.rs) — 用有效配置检查并冻结完整证据预算；探测和实际受理均检查，人工输入在写事件前累计校验，模型调用前复核实际上下文。预算纯函数与边界见 [brain](../brain/index.md)
- [operations/query/chunks.rs](../../crates/worker/src/operations/query/chunks.rs)、[result_reader](../../crates/worker/src/result_reader/mod.rs) — Worker 与稳定 Host 共用持久化结果读取器，校验执行归属并拒绝符号链接；`result`、`team.topic` 分块携带内容摘要 `version`。最多缓存 16 份快照，Team 首次流式散列后按偏移读取，文件替换或原位修改使快照失效；不恢复原 Runtime，也不改写原生结果
- [state.rs](../../crates/worker/src/state.rs)、[state/tests.rs](../../crates/worker/src/state/tests.rs) — 关闭时等待执行和后台任务的整个 future 析构；最后一个 Worker 持有者释放 `NodeLock` 时显式解锁，避免 fork 或复制的文件描述符延长节点目录占用。未获得锁的打开失败不能解锁其他 Worker。
- `runtime/health.rs` 统一计算节点存储准入：可用磁盘块低于 10% 或可用 inode 低于 20% 拒绝新执行；Windows 读取字节容量，inode 字段为 `None`，不伪造值。容量读取失败、零容量仍拒绝准入。健康查询和新执行入口共用纯函数判断，已接收的工作可继续完成。
- `operations/dag_preflight.rs` 使用本次冻结配置校验静态步骤和动态模板。所有 DAG 都要求配套 rootfs、runc 与只读源挂载；Codex Agent 额外校验 guest CLI 与节点登录目录，纯二进制与纯 Codex 不要求原生 provider 凭证。实际执行和私有挂载由 dag-runtime 负责。
- 保留 Runtime 经受控程序升级后，`operations/dag_preflight/image.rs` 只接受维护流程保存的镜像替换回执：原镜像、原 Runtime 数据目录、新私有镜像和运行器摘要必须匹配；随后仍校验完整节点构建信息。普通配置变更不替换已受理镜像。恢复不改原输入、资源、workspace 或历史配置，已完成步骤继续复用。
- [休眠 Runtime 程序修复](../../scripts/platform/rolling/maintenance/engine/repair.py)只使用已完成发布且存储格式兼容的程序。工具核对 Host 休眠记录、预留和原 unit，在 Host 与节点锁内替换程序并保留回退文件；不启动 Runtime。原任务之后仍经原节点的正常恢复入口执行。
- [layout/dag.rs](../../crates/worker/src/layout/dag.rs)、[workloads/dag.rs](../../crates/worker/src/workloads/dag.rs) — 受理保存 UTC 日期目录、资源版本与配置；恢复沿用固定目录与资源。启动先清理遗留容器和挂载，再更新 journal；缺少本次固定数据的未终态运行明确拒绝恢复。
- [operations/query/dag_context.rs](../../crates/worker/src/operations/query/dag_context.rs) — DAG Inspect 从 journal 的冻结定义和原受理目录读取资源快照，校验步骤身份、版本与摘要后投影只读 `context`。未固定为 `preparing`，非待受理运行缺少快照为 `unavailable`，损坏快照明确报错；公开结果不返回宿主路径或 Agent 依赖摘要。
- DAG 的 how 追加由 dag-runtime 写入本地副本；普通 Agent 会话资源追加由 `agent_how.rs` 管理。
- Brain：仅 `brain/v4/`，根节点持有运行、操作与事件投影；`layer` 是已派发层。每层并行执行，全部终态后唤醒决策；人工输入也唤醒一次决策，并取消正在生成的旧决策。层屏障未满足时只可 `guide`，引导动作经 `layered_guidance` outbox 按事件序列投递和确认；模型可依据证据选择正常前进或任一已执行层，内部路径约束由计划准入补齐，回退消耗轮次；末层达标收口。generation 栅栏保障恢复与重复回执幂等。
- [brain/v4/output.rs](../../crates/worker/src/brain/v4/output.rs) 校验冻结的必填输出，保存 `scheduler_output`、`scheduler_artifacts` 并限制两者合计 16 KiB；[brain/output](../../crates/worker/src/brain/output.rs) 为 DAG 步骤、动态实例及其他能力工作区建立文件引用，核对声明大小与 SHA-256，较大索引与嵌套 Brain 使用不可覆盖的清单引用。[operations/artifacts.rs](../../crates/worker/src/operations/artifacts.rs) 提供所属执行文件及清单读取，拒绝内容不符与路径逃逸。[brain/v4/api.rs](../../crates/worker/src/brain/v4/api.rs) 校验 context 中的文件执行归属，并将不可重试的派发拒绝和具体原因原子写入根投影。[workloads/dag.rs](../../crates/worker/src/workloads/dag.rs) 冻结 Brain 的完整 `layered_inputs`，恢复拒绝输入漂移。约定及原生循环测试见 [brain](../brain/index.md)。
- Team 的 `steer` 命令按 `input_id` 去重并写入执行 journal，保留最近 32 条引导；[workloads/team.rs](../../crates/worker/src/workloads/team.rs) 在下一次成员发问时读取，正在生成的成员回答不会被打断。每次讨论回答关闭任务完成后的本地记忆维护，避免为中间发言额外启动模型轮次。
- 上限（`opencoder_brain::layered` 纯域校验）：`LAYERED_MAX_NODES=256`、`LAYERED_MAX_LAYER_WIDTH`=32/层、`LAYERED_MAX_DEPTH=3`、每节点恰好一个能力。Worker 持有模型决策、投影及 generation 栅栏；Control 解析上下文并执行准入。伪造或越权派发帧被拒绝，迟到回执不推进当前尝试。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [DAG 执行约定](../../rules/04-dag-execution-contract.md)
- [agents/brain](../brain/index.md)
- [运行协议](../../docs/brain-orchestration.md)、[动态步骤接口](../../docs/dag-dynamic.md)

## 私有任务文件

DAG 私有文件：准入校验期限、定义及运行中节点执行文件摘要；执行与恢复再次核验摘要。`workloads/dag.rs` 将 owner 私有目录传给 DAG runtime，journal 以 0600 持久化。普通能力探测不计算执行文件摘要，显式私有探测使用异步阻塞任务。`tests/private_files.rs` 覆盖冻结、重放、重启、漂移拒绝与公开读面隔离。

[inventory/](../../crates/worker/src/inventory/mod.rs) 维护五字段索引。会话和项目运行只查询所需列，数据库变更游标、journal 修订和活动运行集合共同更新缓存；常规轮询只处理变化，首次与每 10 分钟校准。`/inventory/changes` 使用 generation 和 revision，按 256 项固定快照分页。

[journal.rs](../../crates/worker/src/journal.rs) 常驻调度摘要与必要的上报字段，完整记录按需读取；读取缓存同时限制总字节和条数。[brain/outbox.rs](../../crates/worker/src/brain/outbox.rs) 先从待确认集合和事务队列选择最多 64 项，再生成消息。终态已确认历史不再参与轮询；派发意图最多复用一个待确认根运行的同一版本，切换或待确认队列清空时释放。ACK 写入独立小表并单调推进，写格式 1 不读取或改写完整执行文件；写格式 0 的过渡版本同时保存旧标记。引导按序发送，数据库提交与队列变更同事务，重启恢复只在启动时补齐旧记录。

执行记录的摘要、按需读取、缓存上限与并发一致性见 [journal](journal.md)。
