# 模型选择与会话覆盖契约冻结

一句话：冻结并实现显式模型优先级、Shared 会话选择及新建会话模式元数据的统一契约；
同步固定 Env 单次解析、Operator 配置冻结与失败回滚边界。

## 1. 显式模型优先级与清理语义

以下优先级从高到低，并已在 startup、headless、task Resume/Fork 中保持一致：

1. `--model`
2. `OPENCODER_MODEL`
3. Resume/Fork 持久化的模型选择
4. per-mode config

显式模型覆盖生效时，必须清除会话级 Shared、Act、Plan 的模型选择，但保留生成参数和
context overlay；清理范围不得扩大到这些非模型配置。legacy 会话的 `NULL` 元数据继续
保持 legacy 语义，不因显式模型覆盖或恢复流程被强制转换为新模式。

## 2. Shared 菜单选择与 `SessionModelOverrides`

Shared 菜单中的选择必须立即成为当前有效的 Shared 模型；即使配置中存在
`modes.shared.model`，会话级 Shared 选择仍然优先生效。

`SessionModelOverrides` 增加可选的 Shared 选择，其语义冻结为：

- `{}`（包括 Shared 字段缺省）：没有会话级 Shared 选择，使用配置 overlay。
- Shared 字段有值：该会话选择优先于 `modes.shared.model`，并立即作为有效 Shared
  模型。
- 整体为 `NULL`：保持 legacy 语义，不按模式感知覆盖解释。

既有 `{}` 必须可直接反序列化为“无 Shared 会话选择”；新增 Shared 字段不额外提升数据库 schema 版本。
上述选择、优先级和清理规则必须同样适用于 resume、fork、new、显式模型覆盖及 Env
reload，不得因入口不同而产生分歧。

## 3. 新建会话统一为模式感知

所有生产路径的新会话创建者（包括 `SessionState::new`、Web、TUI/Local、Node、TODO、
Project、DAG 与 subagent 创建路径）必须显式以 `Some(default())` 初始化模式模型覆盖，
成为 mode-aware 会话，不得默认写成 `NULL`。底层 `Store::create_session` 保留既有
legacy `NULL` 默认；需要模式感知语义的生产调用方必须显式传入 `Some(default())`。

历史数据库行、旧 bundle 与显式历史/legacy import 继续以 `NULL` 表示 legacy 语义；
底层 legacy 创建接口也继续使用 `NULL`。正常生产新建路径不得把 `NULL` 作为默认状态。

## 4. Env 单次解析与 Operator 冻结

严格 Config 加载与 `EnvAware`/`ActiveEnv` 保存各自在操作开始时只解析一次 active Env，
config 与 CLI/Skill 领域文件共用该结果。stale marker 保留非 Env 回退；marker 或 Env
目录的非 `NotFound` I/O 错误在加载或任何写入前 fail closed。显式 `Some(home)`、
frozen 与 operator 加载不读取交互式活动 Env。

Operator 需要补建配置时，由一次严格 Config 操作同时返回 effective Config 与同一批读取
得到的 raw-domain 值，避免 Config 和领域文件来自不同快照。完整 bootstrap 成功后才发布
私有 `.bootstrap-complete`；marker 存在即冻结，后续调用不读取 live Config，也不补齐有意
缺失的领域文件。marker 缺失时，私有、按目录隔离的 `.bootstrap-lock` 串行化 bootstrap
尝试，并在获锁后复查 marker；若仍缺失才继续。失败可重试，已存在文件保持字节不变，
不以新的 live 视图覆盖。

## 5. 运行时同步与懒行回滚

task Resume/Fork 先恢复持久化选择，再重应用进程级显式胜者，并从覆盖后的 effective
config 构造 client。只有恢复、client 构造、模型持久化和 worker 应用成功后，才提交外层
config/client/chat/context；失败保留原运行时。模型与元数据分步写入时，第二步失败补偿
第一步，并恢复相应内存状态。

新任务继承与启动显式覆盖不提前物化会话行。懒行确需由模型操作物化时，以最终模型和
元数据一次插入；后续写失败只回滚确认由本次新建的行，已有行只更新且绝不作为补偿删除
对象，回滚失败明确报告可能残留。

## Implementation status: complete

- 上述模型、会话元数据、Env、Operator 与回滚契约均已实现。
- Shared 字段未额外提升数据库 schema 版本；未新增公共 HTTP 路由或公共 CLI 命令域。

## Verification

以下为本次实现的定向实跑记录；未运行 `cargo test --workspace` 或其他全量 workspace 测试套件。

### 定向测试（通过）

- **Core**（`cargo test -p opencoder-core`，按 target/filter 分组）：`local_env` 解析 2；
  `--test config_contract` 22；`--test config_local_env` 6；
  `--test config_local_env_loading` 5；`--test config_modes` 6；`snapshot`/`domain`
  定向覆盖；`keymap` 6；`mcp` 17。
- **Store**：`cargo test -p opencoder-store --test session_model_overrides` 6；
  `--test store_integration bundle` 8；`--test node_tasks_query dispatched_session_starts_with_empty_mode_aware_metadata` 1。
- **Session**：`cargo test -p opencoder-session --test mode_model_persistence` 15；
  `--test mode_model_requests` 5，覆盖 Shared 生命周期及懒行创建/失败回滚。
- **Web**：`cargo test -p opencoder-web --test session_model_metadata` 2。
- **TUI**（`cargo test -p opencoder-tui`，按 target/filter 分组）：`app_task` 14；
  `model_session_switch` 8；`model_menu`、`session_only`、`model_outcome`、`worker`、
  `ts_mirror` 定向组；集成 target `model_switch_persist`、`session_switch_restore`、
  `resume_context_used`；`app_bootstrap` 3；`handle_mcp_outcome` 6；remote session/subtask
  metadata 定向组。
- **Local**：`cargo test -p opencoder-local model_override` 8。
- **Worker**（`cargo test -p opencoder-worker`，按 filter 分组）：Operator bootstrap 6、
  配置 fingerprint 4、configuration state 13、worker agent metadata 1。
- **生产元数据**：Node、TODO、Project、DAG 与 subagent 的 focused production-metadata
  测试均通过，确认各生产创建路径写入 mode-aware 元数据。
- **Ontology**：`cargo test -p opencoder-control --test ontology_catalog platform_tui_entry_synchronizes_interactive_env_and_mode_model_configuration -- --exact`（1），
  覆盖 `platform:tui` 精确用例。

### 编译、lint 与格式（通过）

- 所有受影响 crate 的 `cargo check --tests` 通过。
- Clippy 均以 `-D warnings` 通过：core/store/session/web/local/tui/worker；
  node/todos/project/dag-runtime；以及 control 的 `--test ontology_catalog` target。
- `cargo fmt --all --check`、`git diff --check` 通过；行数审计通过（迭代文件均
  ≤800 行，新文件均 ≤400 行）。

### 既有失败（不计入通过项）

`cargo test -p opencoder-worker --test operator_initialization_recovery operator_recovery_completes_each_initialization_stage_and_keeps_server_profile -- --exact`
仍失败，错误为 `400 agent codexops unavailable`。该用例在当前树与干净 HEAD `4792d1c9d`
对照复跑均同样失败，属于既有的、与本次无关的 fixture 前提问题；本次未为此修改生产代码，
也不宣称该用例通过。
