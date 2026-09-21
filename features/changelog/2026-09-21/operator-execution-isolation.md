Commit: 40a688a77bfdbedc3f30f9f6b1e3a1ba67244d68

# Operator 执行隔离补全（P1–P4）

## 变更
- P1 节点级 Operator 配置平面：新增 `crates/worker/src/operations/operator_config.rs`。首个 Operator 执行准入时把节点 live 配置（`config.json` + 五个域文件）以 0600 first-writer-wins 引导到 `<data>/operator-config/`，之后冻结；`state.rs::configuration_for`（Operator → `load_operator`）接入准入路径。TUI/CLI 对共享 workdir 配置的保存不再触达 Operator 执行。
- P2 执行级技能池隔离：`core::skill` 增加任务本地执行根（`execution_root`/`with_execution`），`skills_dir()` 优先级为执行根 → 节点 pinned 根 → 真实 home；`freeze_skills` 把 `<data>/operator-config/skills/` 包 + 内置 seed 写入执行 home，用户全局池永不为来源；`operations/launch.rs` 以 `with_execution` 包裹 workload。
- P3 env-freeze 语义：`Config::load_with_home_frozen`（跳过 `apply_env`，快照即最终）用于版本化 Operator resume（`brain/workdir.rs::execution_config`）；新增 `Config::load_operator` 与 `effective_domain_value`/`domain_file_for`。
- P4 会话泳道：schema v27→v28 增 `sessions.kind TEXT`；创建时定值 `operator`/`agent`/`team`/`dag`/`todos`/`project`/`brain`（存量 NULL）。默认清单 SQL 排除 `kind='operator'`，精确泳道 `kind = ?`；`worker/service.rs::indexes()` 按 `row.kind` 解析已打标行，NULL 行保留 id 前缀/标题回退。

## 修复
- `libsql_store/sessions.rs::row_to_meta` 在插入 `kind` 列后未整体后移字段索引，导致 `get_session` 读串列（快照/summary 等），store bundle 集成测试失败。
- `maintenance_dialogs_clear` 集成测试改用 operator 泳道过滤（默认清单现在排除 operator）。
- 约 80 处测试夹具 `SessionMeta` 初始化补 `kind: None`（store/session/web/tui/local/todos/worker）。

## 测试
- `cargo check --workspace --all-targets` 通过。
- 单元/集成：store、session（857）、web/todos/project/dag-runtime、worker（217）、local/tui/core/brain/node/dag-wasm（2518）、shellguard/llm/dag/server/control/agent/agents/team/cli（1066）全绿。
- e2e：operator_e2e 11/11（含新增 `operator_config_plane_frozen_against_workdir_and_user_pool`：TUI 侧保存 `config.json` 与全局池新增包均不进入平面/快照/执行内 `ls`）、dag_e2e 17/17、todos_e2e、team_e2e、brain_e2e、running_mode_switch_e2e、根包 smoke 全绿。
