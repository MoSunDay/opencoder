Commit: 6c3491fd94ec48f28aa51058feb4dd7c9e3daf83

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 在相邻 crate 定义；Config 供全仓
加载。

## 索引
- [build.rs](../../../crates/core/build.rs) — 构建期提交与干净状态捕获
- [version.rs](../../../crates/core/src/version.rs) — 构建期版本元数据
- [src/message.rs](../../../crates/core/src/message.rs) —
  `Message`/`Role`/`ContentBlock`
- [src/tool.rs](../../../crates/core/src/tool.rs) — `Tool` trait、`ToolContext`、
  `ToolOutput`
- [src/config.rs](../../../crates/core/src/config.rs)、
  [src/config/](../../../crates/core/src/config) — Config 加载与合并
  （`load_with_home`、`load_with_home_frozen`、`load_operator`、
  `effective_domain_value`、`domain_file_for`）
- [src/config/runtime/](../../../crates/core/src/config/runtime) — Agent、DAG、
  Ontology、MCP、CLI、技能等运行配置；
  [mode.rs](../../../crates/core/src/config/runtime/mode.rs) 的
  `Config::effective_for_mode` 按“legacy 顶层 → `modes.shared` → 独立
  `modes.act`/`modes.plan`”解析 `model`、`reasoning_effort`、
  `interleaved_thinking`、`max_tokens`、`context_limit`、`cache_salt`。
  缺少/空 `modes` 保留原配置视图，显式 JSON `null` 删除覆盖并恢复继承。
  进程级模型按 `--model` > `OPENCODER_MODEL` > Resume/Fork 持久化的
  Shared/Act/Plan 会话选择 > per-mode config 决定。`apply_explicit_model`
  清除配置中的模式 `model` 覆盖，会话层同时清空 Shared/Act/Plan 会话选择；
  生成参数与 `context_limit` 等 context overlay 保留。模式感知行显式覆盖后写
  `{}`，legacy 行仍写 `NULL`。
- [src/config/local_env/](../../../crates/core/src/config/local_env/mod.rs) —
  本地命名 Env：`~/.opencoder/envs/<name>/{config.json,cli.json,skills.json}`
  与 `~/.opencoder/active_env`；新建时三个配置文件均初始化为 `{}`。
  默认 `Config::load` 的现有调用方均读取 active Env；每次完整加载只解析一次
  active marker，config 与 CLI/Skill 领域文件共用 `Active`/`NoEnv`/`Stale` 结果。
  `config.json` 候选链中活动 Env 覆盖项目、全局与 XDG，CLI/Skill 等领域文件按
  活动 Env → 项目 → 全局 → XDG 选取一个有效文件，不跨文件逐 key 合并。显式
  `Some(home)`、frozen 与 operator 加载隔离该状态。stale marker 保留非 Env 回退；
  marker 或 Env 目录的非 `NotFound` I/O 错误使严格加载 fail closed，不降级复用
  legacy 配置。激活 preflight 将合法的 `modes: null` 按重置模式块处理，其他畸形
  配置仍拒绝。
- [src/config/persistence/](../../../crates/core/src/config/persistence/mod.rs)、
  [domain.rs](../../../crates/core/src/config/domain.rs) — 模型菜单及 `/cli`、`/skill`
  显式以 `ConfigSaveRoute::EnvAware` 启用 Env 路由；`EnvAware` 与 `ActiveEnv` 各自在
  每次保存的任何写入前只解析一次 active Env，配置与各域目标共用该快照。活动 Env 将
  模型/分模式生成参数、CLI、Skill
  分别路由到对应 Env 文件，无活动 Env 或 stale marker 时回退 legacy `Auto`。`Auto`
  与显式 `Project`/`Global` 不被活动 Env 劫持，`ActiveEnv` 无活动时直接报错；
  `EnvAware` 遇到 marker 或 Env 目录的非 `NotFound` I/O 错误在任何写入前 fail
  closed，不写 legacy 文件。领域文件仅 CLI/Skill 支持 Env，MCP/schedules 保持原目标。
- [tests/config_overlay/](../../../crates/core/tests/config_overlay/main.rs) —
  配置覆盖契约测试
- [identity/permissions.rs](../../../crates/core/src/identity/permissions.rs) —
  共享 HTTP 授权纯函数
- [src/harness/](../../../crates/core/src/harness) — 执行器选择与私有运行态
  （`Harness`、`fresh_runtime`、`matches_requested_env`）
- [harness/settings.rs](../../../crates/core/src/harness/settings.rs) — 类型化
  Codex 设置
- [harness/remote.rs](../../../crates/core/src/harness/remote.rs) —
  `ServerConnection`、`ServerCapability`
- [src/agent/](../../../crates/core/src/agent)、
  [src/skill.rs](../../../crates/core/src/skill.rs) — Agent 注册表与引用卡、
  技能发现；memory 池聚合在
  [agent/memory.rs](../../../crates/core/src/agent/memory.rs)
- [src/skill/seed.rs](../../../crates/core/src/skill/seed.rs) — 内置技能增量 seed
- [tests/skill_contract/](../../../crates/core/tests/skill_contract/main.rs) —
  技能契约测试
- [platform/](../../../crates/core/src/platform/mod.rs) — 本地会话与执行节点
  共用的宿主操作
- [net.rs](../../../crates/core/src/net.rs) — 代理感知 HTTP 客户端构造
- [src/data_dir.rs](../../../crates/core/src/data_dir.rs) — 按工作目录的
  数据目录解析
- [fleet/protocol.rs](../../../crates/core/src/fleet/protocol.rs) — Server/Node
  协议（`PROTOCOL_VERSION`）
- [fleet/release.rs](../../../crates/core/src/fleet/release.rs) — 发布交接
- [fleet/report.rs](../../../crates/core/src/fleet/report.rs) — 节点报告
- [fleet/inventory/](../../../crates/core/src/fleet/inventory/mod.rs) — 有界变更
  历史与分页快照
- [fleet/private_files/](../../../crates/core/src/fleet/private_files) —
  `PrivateExecutionContext` 与不可变私有任务文件
- [src/brain/](../../../crates/core/src/brain) — 版本化计划与编排线协议，
  `layered/` 为分层计划；IO 协议在
  [brain/io.rs](../../../crates/core/src/brain/io.rs)
- [ontology/](../../../crates/core/src/ontology/mod.rs) — 公开能力描述与平台入口
  清单；既有 `platform:tui` 登记 TUI `/env`、`/envs`、`/model`、`/config`、
  `/cli`、`/skill`，并描述 Shared 会话选择的即时生效与优先级。这些交互式斜杠
  命令只复用 `/api/tui` 能力查询，不新增公共 HTTP 路由或 `opencoder-cli` 命令域。
