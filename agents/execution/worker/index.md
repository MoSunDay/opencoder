Commit: d0e87cbca

# worker 模块

节点执行面：受理/恢复执行、资源快照与 workload 适配（lib.rs 模块声明）。

Windows 节点只接受 `ExecutionKind::Operator`，用户边界见
[Windows](../../../features/platform/windows/index.md)。

## 入口与模块
- [lib.rs](../../../crates/worker/src/lib.rs) — Node-owned execution adapters；进程内会话 API
- [service.rs](../../../crates/worker/src/service.rs) — 根执行与会话清单（`indexes()`）
- [operations/](../../../crates/worker/src/operations/mod.rs) — 准入、创建、launch、维护命令与查询
- [workloads/](../../../crates/worker/src/workloads/mod.rs) — agent/team/dag/todos/project 执行驱动
- [journal.rs](../../../crates/worker/src/journal.rs) — 执行记录入口；子模块见 [journal](journal.md)
- [state.rs](../../../crates/worker/src/state.rs)、[layout.rs](../../../crates/worker/src/layout.rs) — 节点状态与执行目录布局
- `crates/worker/src/state.rs` `release_execution_state` — 终态回收执行
  门：lifecycle/preparation 门仅在外部仍持有克隆（Arc 强计数 > 1）时
  保留，同时移除维护注册（其 Drop 释放进程级扩展槽）；回归
  [state/tests.rs](../../../crates/worker/src/state/tests.rs) 的
  `terminal_release_collects_unused_gates_and_maintenance_registrations`
- [state/config_cache.rs](../../../crates/worker/src/state/config_cache.rs) —
  配置缓存键记录每个候选文件的路径/size/mtime，以及 active marker、stale
  状态、活动 Env 文件和 workdir/global/XDG 发现身份；`state.rs` 的
  `configuration_shared` 按完整指纹缓存 effective config 为共享 `Arc`，
  文件、Env 或发现目录变化都会失效。默认 `Config::load` 的所有现有调用方都会读取
  active Env，并非仅交互式缓存加载；显式 `Some(home)`、frozen 与 operator 加载保持
  隔离，harness task-local 作用域旁路缓存、加载错误不缓存。无活动 Env、仅 XDG 提供
  `config.json`/`cli.json`/`skills.json` 时，创建、同长编辑和删除也会刷新缓存。
  回归 `configuration_cache_reuses_the_load_until_input_files_change`、
  `configuration_cache_tracks_active_env_transitions_and_files` 与
  `configuration_cache_tracks_xdg_cli_and_skill_creation_edits_and_removals`。
  active marker 的非 `NotFound` I/O 错误不得命中缓存或回退 legacy，stale 仍按无活动
  Env 处理。候选路径的 `Missing`/`Regular`/`Directory`/`Other` 类型分别入指纹，
  目录与其他非普通文件不会折叠为缺失；元数据或 mtime 读取错误向上传播，禁止复用旧缓存
- [runtime/](../../../crates/worker/src/runtime/mod.rs)、[resources.rs](../../../crates/worker/src/resources.rs) — 容量与健康准入、NFS 资源快照
- [runtime/persistence.rs](../../../crates/worker/src/runtime/persistence.rs) — 持久化闩锁指数退避自愈（纯函数模块）：失败按 1s→60s 封顶排定探测（`backoff`），完整成功的派发周期 `recover` 清节点与项目依赖双闩锁，瞬态 DB/NFS 抖动不再永久停摆派发；闩锁读取统一走 `node_error`/`error` 访问器（own→deps 合并次序归模块所有，snapshot 与受理检查不再直读内部状态）
- [inventory/](../../../crates/worker/src/inventory/mod.rs) — 由 journal、数据库与活动运行合成的增量清单
- [result_reader/](../../../crates/worker/src/result_reader/mod.rs) — 不唤醒归属 Runtime 的持久化结果读取
- [brain/](../../../crates/worker/src/brain/mod.rs) — Brain 运行投影与回执；`brain/v4/` 为节点侧实现
- [maintenance_tools.rs](../../../crates/worker/src/maintenance_tools.rs)、[migration.rs](../../../crates/worker/src/migration.rs) — 维护工具与记录迁移

## 关键接缝
- [operations/queue/](../../../crates/worker/src/operations/queue/mod.rs) — 节点持有的持久待办；节拍纯函数 `dispatch_delay`
- [operations/operator_env.rs](../../../crates/worker/src/operations/operator_env.rs) — Operator HOME/WORKSPACE 隔离
- [operator_env/linux_identity.rs](../../../crates/worker/src/operations/operator_env/linux_identity.rs) — Operator 专用 Linux 系统账号
- [operations/operator_config.rs](../../../crates/worker/src/operations/operator_config.rs) —
  节点级 Operator 配置面；需要补建时，一次严格 Config 操作同时取得 effective Config
  与同源 raw-domain 值，全部配置和领域快照成功后才发布私有 `.bootstrap-complete`。
  marker 存在即冻结并直接返回，不再读取 live Config，也不补齐有意缺失的领域文件；
  marker 缺失时以私有、按目录隔离的 `.bootstrap-lock` 串行化尝试，获锁后复查 marker，
  若仍缺失才继续。失败可重试，已有文件保持字节不变；后续 frozen/operator 加载与活动
  Env、后续进程环境变化隔离
- [operations/fork.rs](../../../crates/worker/src/operations/fork.rs) — 派生会话按父执行类型生成 ID 与资源
- [operations/dag_preflight.rs](../../../crates/worker/src/operations/dag_preflight.rs) — DAG 准入校验（镜像替换见 `image.rs`）
- [operations/query/session/](../../../crates/worker/src/operations/query/session/mod.rs) — 持久受理到会话初始化之间的读取
- [workloads/agent_how.rs](../../../crates/worker/src/workloads/agent_how.rs) — Agent 的 how 追加与结果投影（纯函数）
- [layout/dag.rs](../../../crates/worker/src/layout/dag.rs)、[workloads/dag.rs](../../../crates/worker/src/workloads/dag.rs) — DAG 目录与执行驱动
- [tests/](../../../crates/worker/tests) — 集成测试（Windows 准入见 `windows_operator.rs`）

## 相关
- [agents/node](../../platform/node/index.md)、[agents/dag-runtime](../../workflow/dag-runtime/index.md)
- [DAG 执行约定](../../../rules/04-dag-execution-contract.md)
- [agents/brain](../../orchestration/brain/index.md)
- [运行协议](../../../crates/control/src/api/brain_runs)、[动态步骤接口](../../../crates/dag/src/dynamic.rs)
