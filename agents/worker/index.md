Commit: df06a3b0a177c14b53171818717eaca0e0c35e64

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

Windows 节点只声明并接受 `ExecutionKind::Operator`；其他执行种类在准入时拒绝，不装配 DAG/runc/NFS。依赖与实现通过平台模块隔开；原生验收入口见 [windows_operator.rs](../../crates/worker/tests/windows_operator.rs)，用户边界见 [Windows](../../features/windows/index.md)。

## 索引
- [operations/sandbox_session/mod.rs](../../crates/worker/src/operations/sandbox_session/mod.rs)、[workloads/agent_runc/inputs.rs](../../crates/worker/src/workloads/agent_runc/inputs.rs) — 容器会话的 steer 与 queue 使用原生输入存储，按 `input_id` 去重；运行中引导进入当前循环，排队输入在后续轮次消费，续会话沿用同一执行和上下文。
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- `crates/worker/src/workloads/agent_how.rs`、`agent_runc.rs`（+ `agent_runc/`）— how 契约与 `run_mode: agent` runc 运行时（准入 fail-closed）
- [workloads/agent.rs](../../crates/worker/src/workloads/agent.rs)、[agent_runc.rs](../../crates/worker/src/workloads/agent_runc.rs) — 持久化调用方的 `literal_mentions`；容器通过 staged Harness JSON 继承该策略。宿主首轮使用队列投递，展示原文与模型执行前缀分别保存；托管 Codex 设置仍由 Server 冻结。
- [workloads/agent/initialization.rs](../../crates/worker/src/workloads/agent/initialization.rs) — 首轮投递前补齐 Harness、启动环境、展示策略及 Server 模型设置；已有 session 行也执行该步骤。恢复保留已启动的 Codex 线程和冻结设置，Operator HOME 最后覆盖输入与托管环境。
- `crates/worker/src/operations/` — 准入/launch/维护命令/查询（含 `query/instances/`、`artifacts.rs`、`operator_env.rs` Operator 隔离快照、`operator_config.rs` 节点级 Operator 配置平面）
- [operations/create/preflight_agents.rs](../../crates/worker/src/operations/create/preflight_agents.rs) — 项目 Agent、Team、DAG 的 Agent 依赖提取纯函数；DAG 仅收集 Agent 步骤。
- `crates/worker/src/state.rs`、`src/layout.rs`、`src/journal/` — runtime.db、执行布局与原子落盘（layout 含 `<data>/operator/<id>/{home,workspace}` 预留）
- `crates/worker/src/runtime/`、`src/resources.rs` — Runtime 归属与资源快照；Runtime 启动完成遗留 runc 清理和 journal 恢复后，容量恢复还要证明当前进程属于受管 systemd Runtime 且同一 cgroup 没有其他进程，并确认本进程没有托管子进程，才逐票据释放遗留 `running` reservation；无法证明时 fail-closed。
- `crates/worker/src/brain/` — schema 7 里程碑投影、模型激活、回执及唤醒；`brain/v4/` 实现节点调度，`workdir.rs` 提供能力工作空间接缝
- `tests/` — 集成测试

## Operator 执行隔离

配置平面：`operations/operator_config.rs` 维护节点数据根下的 `<data>/operator-config/`（`config.json` + `mcp|cli|skills|ap|schedules` 五域文件 + `skills/` 技能包目录）。首个 Operator 执行准入时（`state.rs::configuration_for` → `operator_configuration`）从节点 live 视图 `bootstrap` 一次，以私有权限按 first-writer-wins 发布，此后冻结：TUI/CLI 对共享 workdir 配置的保存不再进入 Operator 执行；`freeze_skills` 把平面 `skills/` 包 + 内置 seed 写入执行 home（用户全局池永远不是来源）；`operations/launch.rs` 用 `core::skill::with_execution` 把 workload 包进执行技能根。

`operations/operator_env.rs` 为 `ExecutionKind::Operator` 提供按执行的 HOME/WORKSPACE 隔离：`materialize()` 在准入通过后把冻结配置快照（明文含 provider api key）以私有权限写入 `<data_dir>/operator/<id>/home/.opencoder/config.json`，`resolve()` 核验快照与 workspace。`env_pairs()` 产出 HOME 覆盖对，Windows 同时隔离 USERPROFILE、APPDATA、LOCALAPPDATA；fresh 会话在输入 envs 之后注入，随后经 harness envs 持久化，resume 由 `resume.rs` 重建 env_passthrough。`operations/create.rs` 准入接受 Operator 的显式 Harness 选择，Codex 预检合并托管设置与输入 envs；`workloads/agent.rs` 固定所选 Harness 和环境。配置加载走 core `Config::load_with_home`，web drain 栈经 `AppState.config_home` 穿参，执行目录由 `brain/workdir.rs` `session_dirs()` 裁定。Maintenance/Agent/Brain 不受影响。

会话泳道：Operator 执行的 Primary Session 创建时打 `kind='operator'`（其他 kind 同理，见 store 索引），默认清单泳道排除 operator 行；`service.rs::indexes()` 按 `row.kind` 精确解析已打标行，存量 NULL 行保留 id 前缀/标题回退。Agent 和 Operator 空闲终态均从持久化会话取最后一条非空 assistant 文本，以有界 `output_text` 写入执行结果；Maintenance 保留会话指针结果。

## 接缝
- [brain/v4/budget.rs](../../crates/worker/src/brain/v4/budget.rs) — 用有效配置检查并冻结完整证据预算；探测和实际受理均检查，人工输入在写事件前累计校验，模型调用前复核实际上下文。预算纯函数与边界见 [brain](../brain/index.md)
- [operations/query/chunks.rs](../../crates/worker/src/operations/query/chunks.rs)、[result_reader](../../crates/worker/src/result_reader/mod.rs) — Worker 与稳定 Host 共用持久化结果读取器，校验执行归属并拒绝符号链接；`result`、`team.topic` 分块携带内容摘要 `version`。最多缓存 16 份快照，Team 首次流式散列后按偏移读取，文件替换或原位修改使快照失效；不恢复原 Runtime，也不改写原生结果
- [state.rs](../../crates/worker/src/state.rs)、[state/tests.rs](../../crates/worker/src/state/tests.rs) — 关闭时等待执行和后台任务的整个 future 析构；最后一个 Worker 持有者释放 `NodeLock` 时显式解锁，避免 fork 或复制的文件描述符延长节点目录占用。未获得锁的打开失败不能解锁其他 Worker。
- `runtime/health.rs` 统一计算节点存储准入：可用磁盘块低于 10% 或可用 inode 低于 20% 拒绝新执行；Windows 读取字节容量，inode 字段为 `None`，不伪造值。容量读取失败、零容量仍拒绝准入。健康查询和新执行入口共用纯函数判断，已接收的工作可继续完成。
- `operations/dag_preflight.rs` 使用本次冻结配置校验静态步骤和动态模板。所有 DAG 都要求配套 rootfs、runc 与只读源挂载；Codex Agent 额外校验 guest CLI 与节点登录目录，纯二进制与纯 Codex 不要求原生 provider 凭证。实际执行和私有挂载由 dag-runtime 负责。
- [layout/dag.rs](../../crates/worker/src/layout/dag.rs)、[workloads/dag.rs](../../crates/worker/src/workloads/dag.rs) — 受理保存 UTC 日期目录、资源版本与配置；恢复沿用固定目录与资源。启动先清理遗留容器和挂载，再更新 journal；缺少本次固定数据的未终态运行明确拒绝恢复。
- [operations/query/dag_context.rs](../../crates/worker/src/operations/query/dag_context.rs) — DAG Inspect 从 journal 的冻结定义和原受理目录读取资源快照，校验步骤身份、版本与摘要后投影只读 `context`。未固定为 `preparing`，非待受理运行缺少快照为 `unavailable`，损坏快照明确报错；公开结果不返回宿主路径或 Agent 依赖摘要。
- DAG 的 how 追加由 dag-runtime 写入本地副本；普通 Agent 会话资源追加由 `agent_how.rs` 管理。
- Brain：仅 `brain/v4/`，根节点持有运行、操作与事件投影；`layer` 是已派发层。每层并行执行，全部终态后唤醒决策；人工输入也唤醒一次决策，并取消正在生成的旧决策。层屏障未满足时只可 `guide`，引导动作经 `layered_guidance` outbox 按事件序列投递和确认；模型可依据证据选择正常前进或任一已执行层，内部路径约束由计划准入补齐，回退消耗轮次；末层达标收口。generation 栅栏保障恢复与重复回执幂等。
- [brain/v4/output.rs](../../crates/worker/src/brain/v4/output.rs) 校验冻结的必填输出，保留完整结构化决策证据；[brain/v4/api.rs](../../crates/worker/src/brain/v4/api.rs) 将不可重试的派发拒绝和具体原因原子写入根投影。[workloads/dag.rs](../../crates/worker/src/workloads/dag.rs) 从 Brain 的 `layered_inputs` 读取参数。约定及原生循环测试见 [brain](../brain/index.md)。
- Team 的 `steer` 命令按 `input_id` 去重并写入执行 journal，保留最近 32 条引导；`workloads/team.rs` 在下一次成员发问时读取，正在生成的成员回答不会被打断。
- 上限（`opencoder_brain::layered` 纯域校验）：`LAYERED_MAX_NODES=256`、`LAYERED_MAX_LAYER_WIDTH`=32/层、`LAYERED_MAX_DEPTH=3`、每节点恰好一个能力。Worker 持有模型决策、投影及 generation 栅栏；Control 解析上下文并执行准入。伪造或越权派发帧被拒绝，迟到回执不推进当前尝试。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [DAG 执行约定](../../rules/04-dag-execution-contract.md)
- [agents/brain](../brain/index.md)
- [运行协议](../../docs/brain-orchestration.md)、[动态步骤接口](../../docs/dag-dynamic.md)

## 私有任务文件

DAG 私有文件：准入校验期限、定义及运行中节点执行文件摘要；执行与恢复再次核验摘要。`workloads/dag.rs` 将 owner 私有目录传给 DAG runtime，journal 以 0600 持久化。普通能力探测不计算执行文件摘要，显式私有探测使用异步阻塞任务。`tests/private_files.rs` 覆盖冻结、重放、重启、漂移拒绝与公开读面隔离。
