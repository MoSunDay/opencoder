Commit: c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6（开发基线；多轮工作树成果随本提交落地）

# 项目 TODO 多执行器挂载：agent/team/dag/brain 四执行器与大脑路由

## 问题与背景

- 项目面板 todo 此前仅由单 agent（act 会话）执行；本轮把 todo 执行维度扩展为 `executor_kind: agent | team | dag | brain`，`executor_ref`（team/dag 名或 brain 固定能力 ID）与 `executor_spec`（team 成员表 `TeamSpec` / DAG spec / brain 路由表 `BrainRoutes`）随 todo 持久化。plan 阶段仍由 plan agent 生成方案，`plan_md` 与执行器解耦（plan run 的 `executor_kind` 恒为 agent）。
- 存储迁移 SCHEMA_VERSION 19→20：todos 表新增 `executor_kind TEXT NOT NULL DEFAULT 'agent'`、`executor_ref`、`executor_spec`；project_todo_runs 新增 `executor_kind`、`capability_id`、`plan_id`、`output_ref`；CREATE TABLE 引导 DDL 同步。

## 设计要点

- **纯类型放 store**：`TeamSpec`/`BrainRoutes`/`validate_spec` 落在 `crates/store/src/project_executor_spec.rs`（P0 拆分下 control 不得依赖 opencoder-project，但仍需校验入参）；project crate 以 `executor::spec` 再导出；store 为此依赖 opencoder-dag（复用 DAG spec 与无环校验）。
- **run 行记录解析后执行器**：brain todo 执行时才解析为具体 agent/team/dag，run 行落解析后的 kind 与溯源（`capability_id`/`plan_id`）；`output_ref` 承载外部工件引用（dag=workflow 工件根路径，team=topic ULID）；执行器标签 `team:{name}` / `dag:{name}` / `brain`。
- **project executor 驱动**（`crates/project/src/executor/`，原 execute.rs 拆为 agent_drive）：agent=既有 act 会话 resume；team=`LocalTeamDispatcher` 每 ask 一会话、物化团队名 `project-{清洗后 todo id}`、team_root 取 `data_dir_for(workdir)/team`、FINISH_* 事件映射 run/todo 终态；dag=`Uplink::for_local_dag` + 本地事件写 session_events、workflow_root `<data_dir>/workflow`、run_id 直接用项目 run id（`prun-<ULID>` 满足 validate_run_id）。
- **大脑路由双通道**：本地 web 走 `Deps.brain`（`dispatch_or_plan`，situation=目标链+标题+草稿+方案；`executor_ref` 固定能力；`executor_spec`=BrainRoutes，默认 {agent,"act"}）；平台侧 control `brain_preresolve` 预解析（fleet `capability_target` 绑定）后经 `CreateExecution.input["brain"]` 下发覆盖；worker 读 `input["brain"]` 或 `record.result["brain"]`（恢复/重跑路径）→ `start_execute_with(todo_id, ExecutorOverride)`（serde 字段名 `ref`）。
- `ProjectService::init` 增第 5 参 `brain: Option<opencoder_brain::Runtime>`（web 传 Some，节点 worker 传 None——节点不做大脑路由；brain `Runtime` 派生 Clone）。
- worker 预检（`operations/create.rs`）按 kind 聚合所需 agent 清单：agent→todo.agent；team→spec 成员；dag→spec 的 agent step；brain→override 的 ref/spec；spec 缺失时跳过。

## 行为面与测试

| 行为面 | 关键测试 | 文件 |
| --- | --- | --- |
| store 执行器维度 CRUD 往返 + v19→v20 迁移（手写 v19 库重开升 20，列齐） | `executor_dimension_round_trips`；`schema_migration_v19_to_v20_adds_project_executor_columns` | `crates/store/tests/project_store.rs`、`crates/store/tests/store_migrations/catalog.rs` |
| team/dag/brain 驱动集成：dag 内联 spec 执行+工件 output_ref、team 内联 spec 跑完、brain 固定能力路由到已注册 dag、brain 无 Runtime 在 claim 前明确报错（4 例） | `dag_executor_runs_inline_spec_and_writes_artifacts`、`team_executor_runs_inline_spec_to_completion`、`brain_pinned_capability_routes_to_registered_dag`、`brain_without_runtime_is_rejected_before_claim` + 既有 plan/execute 生命周期回归 | `crates/project/tests/executor_team_dag_brain.rs`、`crates/project/tests/plan_and_execute.rs`、`crates/project/src/service_tests.rs` |
| web API executor 字段创建/PATCH 往返（双 option null 清除）、非法 kind/spec 400、plan run 落 agent（新增 1 例） | `todo_executor_fields_create_patch_and_run_shape` | `crates/web/src/api_project_todos.rs`、`crates/web/tests/web_project.rs` |
| control 大脑预解析：capability_target 绑定映射 + 非执行 kind 拒绝（纯函数单测） | `brain_override_maps_targets_and_carries_provenance`、`brain_override_rejects_non_executor_kinds`（含 `brain_preresolve` 接线） | `crates/control/src/api/project.rs` |
| worker 覆盖注入与重跑再水化（input/result 的 brain → ExecutorOverride）+ kind 感知预检 | `preflight_agents_follow_the_executor_kind` 及既有 worker 套件 | `crates/worker/src/workloads/project.rs`、`crates/worker/src/operations/create.rs` |
| SPA 执行器选择/动态字段/列表列/run 徽标（executor/capability/plan/output_ref 截断）（7 例） | `tags every row with its executor kind (+ ref/agent secondary text)`、`offers the four executor kinds; conditional fields follow the pick`、`submits the executor slice with the POST body (no agent key for team)`、`rejects an invalid spec JSON locally without POSTing`、`show the resolved executor kind, brain provenance and output_ref`、`executorBody trims, omits blanks and gates on JSON validity`、`executorRefText falls back to the agent name / blank` | `crates/web/spa/src/project/todosTab.dom.test.jsx`、`todosTab.jsx`、`todoDrawer.jsx`、`labels.jsx` |

## 验收结果

- `cargo test -p opencoder-store`：228 passed / 0 failed（含本轮新增迁移与执行器维度用例）。
- `cargo test -p opencoder-project`：37 passed / 0 failed；与 store 合跑 265 passed / 0 failed。
- `cargo test -p opencoder-web`：276 passed / 0 failed；`cargo test -p opencoder-control`：197 passed / 0 failed（unit 36 + e2e 159 + admission 2）；SPA `npx vitest run`：457 passed（52 个测试文件）。
- 全量 `cargo test --workspace --no-fail-fast`：**4780 passed / 5 failed**；5 例失败全部位于 opencoder-worker 的 wasm 迁移测试（`artifact_stream::streams_256_mib_artifact_with_bounded_frames_and_memory`、`durable_lifecycle::completed_execution_wins_a_late_cancel_without_rewriting_results`、`workloads::dag_artifacts_and_checkpoints_survive_node_restart`、`workloads::dag_cancel_interrupts_wasm_step_and_releases_node_capacity` 等），归属并行推进的 python→wasm 工作流改造（失败用例由该工作树改造为 `stage_stdout_wasm`/`"kind":{"type":"wasm"}`，见 `crates/worker/tests/artifact_stream.rs` 与 `tests/support/wasm`），与本轮 project 执行器改动无关（本轮 worker diff 仅 project 工件路径与 brain 覆盖注入）。
- 行数预算：新文件 ≤400（`sql_store/project_crud_todo.rs` 327、`project_executor_spec.rs` 251、executor 四驱动 246/229/270/361、`todosTab.dom.test.jsx` 205）；迭代文件 ≤800（`service.rs` 449，测试拆至 `service_tests.rs` 442）。

## 评审整改（同日第二轮）

评审发现平台 brain 主路径断裂（D1）及其余低危项，本轮全部整改并重跑门禁：

- **D1（P0）brain 覆盖透传**：claim 前解析出的 `ExecutorOverride`/`BrainTrace` 原先在派发时被丢弃（`service.rs` 只派发裸 Brain 标记、`executor::drive` 丢弃 `brain_trace`、`brain_drive` 以 `override_=None` 重解析），节点（`Deps.brain=None`）上未钉住的 brain todo 必然 run/todo 双 Failed。修复：新增 `BrainHandoff { override_, trace }` 随 `executor::drive` → `brain_drive::drive` 透传，驱动内重解析直接采纳 override 纯函数分支（节点无 brain 运行时也可执行），留痕沿用 claim 前解析（单一事实源）；`resolve_brain` override 分支补拒 brain→brain（镜像 `executor::resolve`）。
- **D3 四处**：① dag 宿主 session 创建前的失败不再写 `session_id`（`DagFailure { error, host_session }` 区分前后）；② `run_agent_label` Agent 标签用解析后 ref（无 ref 回落 `todo.agent`）；③ web kind-only PATCH 复验存量 spec（effective spec = 补丁 spec 或现存 spec，防旧 spec 借换 kind 走私）；④ worker 预检 brain 覆盖改 result-first（`mirror_result_overrides`，与执行面 `brain_override` 同序，旧 input 键不再遮蔽再执行解析）。
- **D2 MySQL/StarRocks 升级路径**：`ddl::upgrade` 按 `information_schema.columns`（经 `exec_read_all`，StarRocks 全语句 text 协议防陈旧快照）补 `ADD COLUMN`，既存部署不再首次写入 `Unknown column`；`UPGRADE_COLUMNS` 与 CREATE 常量有 drift 测试钉死。
- **D4**：spec 校验补重复成员 node_id / 成员重复 capability / 路由表重复 capability 拒绝；`schema.rs` 803 行拆出 `schema_tests.rs`（755 行）；control `brain_preresolve` 空能力库按 dispatch 面同款回落默认 agent（不再 400）、`get_todo` Err 不再吞（500）；`load capability target` 瞬态错误 500→503。

整改新增测试：`brain_override_drives_on_node_without_runtime`、`brain_override_of_kind_brain_is_rejected_before_claim`、`dag_early_failure_leaves_no_dangling_session_id`、`run_agent_label_prefers_resolved_ref_over_todo_agent`（project）；`patch_kind_only_revalidates_stored_spec`（web）；`result_brain_mirrors_over_stale_input_brain`（worker）；`brain_todo_execute_preresolves_empty_library_to_default_agent`（control e2e）；`upgrade_alters_carry_backend_text_type_and_full_definitions`、`upgrade_columns_do_not_drift_from_create_table_consts`、`missing_columns_drives_idempotent_upgrade_shape`、`column_name_extracts_first_whitespace_token`、`validate_spec_rejects_duplicate_team_members_and_capabilities`、`validate_spec_rejects_duplicate_brain_route_capabilities`（store，mysql/starrocks feature）。

整改后门禁（2026-09-08）：`cargo test --workspace --locked` **4796 passed / 0 failed**（首轮基线 4780/5 中的 5 例 wasm 失败已由并行 wasm 工作流在其推进中自行收敛）；分 crate：store 230（另 feature 门禁 46）、project 42、web 278、control 198、worker 71；`cargo fmt --check` 与 `cargo clippy --all-targets -D warnings` 对本轮触碰文件全绿。

## 评审残留收口（第三轮）

关闭第二轮评审残留 TODO（R1/R2/R3）：

- **R1（P2）MySQL/StarRocks 升级契约 live 用例**：新增 `store/tests/sql_project_upgrade.rs`（`OC_TEST_MYSQL_DSN` / `OC_TEST_STARROCKS_DSN` 门控，无 DSN 跳过）：预建 pre-executor 旧表形（硬编码历史列集，fixture 自证无 executor 列）→ `sql_store::open`（apply 对既有表 no-op + upgrade 补列）→ `information_schema` 断言 3+4 列补齐（StarRocks 全程 text 协议 + eventually 轮询）→ 升级后经 `Arc<dyn ProjectStore>` 走 executor 维度 CRUD（todo kind/ref/spec 建查改、run capability/plan/output_ref 建 patch 查、级联删）→ 二次 open 幂等 no-op。StarRocks `information_schema` 可见性 / `DATABASE()` / PK 表 ADD COLUMN 由该契约钉死（有 live 环境即真跑）。
- **R2（P3）control `get_todo` Err→500 分支补测**：`bootstrap.rs` 增加 `new_state_with_projects` 注入缝（生产路径 `new_state` 不变），e2e `Harness::with_projects` 透传；`project_store_failure.rs` 以全委托包装 store（仅 `get_todo` 注入 Err）断言 `POST /api/project/todos/:id/execute` → 500 `{"error":"load todo: injected get_todo failure"}`（preresolve 先于 fleet.index，无其他面被牵动）。
- **R3（P3）override 路径跳过重复留痕写**：`brain_drive.rs` 的 brain trace stamp 补 `handoff.override_.is_none()` 门——override 随行时留痕已由 claim 前 INSERT 随 run 行落库（单一事实源），跳过同值幂等 `patch_todo_run`；本机 brain 路径（无 override）仍按驱动内重解析刷新。端态契约由既有 override 测试钉住（值改经 claim 前 INSERT 落库）。
- 顺手项：feature 门控 clippy 暴露的既有 `project_text_chunk` 8 参 lint 补注释化 `#[allow]`（参数表钉死于 `ProjectStore` trait 缝）；`agents/store/index.md` 代表性验证补升级契约条目、`agents/control/index.md` e2e 用例数 159→161（含本轮 +1 与并行 wasm 工作流 +1）并记 projects 注入缝。

### 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| MySQL 旧表 → upgrade → CRUD 升级契约 | `mysql_project_upgrade_contract` | `store/tests/sql_project_upgrade.rs` |
| StarRocks 旧表 → upgrade → CRUD 升级契约 | `starrocks_project_upgrade_contract` | `store/tests/sql_project_upgrade.rs` |
| control todo store 故障 execute → 500 | `execute_reports_500_when_todo_store_fails` | `control/tests/e2e/project_store_failure.rs` |
| override 留痕端态不回退（claim 前 INSERT 单源） | 既有 `brain_override_drives_on_node_without_runtime` | `project/tests/executor_team_dag_brain.rs` |

### 门禁

- `cargo test --workspace --locked` → **4797 passed / 0 failed**（331 个结果块全 ok；上轮 4796 + control e2e 新增 1；store 升级契约 2 例为 feature 门控，不计入默认全量）。
- `cargo test -p opencoder-store --features mysql,starrocks` → 全绿（无 DSN 环境下两例契约按设计跳过并告警）。
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告（含 store 双 feature 门控跑）。
- `cargo build --workspace` → 零错误。行数：新文件 399/185 ≤400，触碰文件均 ≤532。
- 流程备注（承第二轮 R4）：全量仍跑在含未提交并行 wasm 改动的共享树上（本次运行还编译了其新增 example target）；wasm 工作流定稿提交前，合入门禁以其后重跑的全量为准。
