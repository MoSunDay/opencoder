# 移除 autopilot 功能并收口 SPA 悬空入口

一句话：autopilot（session 级自动 plan/act/review 循环）从全仓下线——
session 引擎、core 配置域（`autopilot` 键 / `ap.json`）、store 列与迁移、
TUI `/ap` 命令、`ap_menu` 与 `/config` 面板的 `ap max_iter` 字段、web
`POST /api/sessions/:id/autopilot` 路由、ctl 展示、e2e/acceptance 脚本中的
autopilot 场景、`SessionEvent::AutoPilot` 一并删除；SPA 侧同步移除指向已删
端点的 `/ap` 命令条目、autopilot 弹窗与 `setAutopilot`（此前后端先删、SPA
未同步，`/ap` 必 404），`dist/static/app.js` 随源码重建。
`reduce.js` 的 `autopilot` 帧渲染臂保留：历史会话的 SSE 回放仍会透传旧帧。

> 后续修正（2026-11-24）：提交树中的 `dist/static/app.js` 仍含该回放臂（源码已无），已重建移除；运行时 `ap.json` 与 `config.json` 的 `autopilot` 键同步清理，见 [remove-autopilot-leftovers.md](../2026-11-24/remove-autopilot-leftovers.md)。

旧行为降级（均为刻意）：
- 旧 config.json 的 `autopilot` 键按未知字段忽略，不再解析、不外露；
  `ap.json` 退出域文件表（`DOMAIN_FILES` 5 → 4），磁盘遗留文件不再加载。
- store：`SessionMeta`/`SessionPatch` 去掉 `autopilot_mode` /
  `clear_autopilot_mode`，建表与增删改查 SQL 同步去掉该列；v11 加列迁移删除，
  新增 v40 迁移（`column_exists` 守卫）删除旧库遗留的 `sessions.autopilot_mode`
  列，旧库升级后行数据保留（schema watermark 39 → 40）。
- `SessionEvent::AutoPilot` 变体删除后，TUI 回放 `deserialize_event` 对旧负载
  反序列化失败按 `warn + 跳过` 处理，不中断整段回放；web SSE 历史按
  `payload + sse_kind` 原样透传，SPA 渲染臂保留，旧会话可继续回放。
- `resume.rs` 不再读取 `sessions.autopilot_mode`，旧库中的会话级覆盖随 v40 列删除失效。
- `handoff.rs` 仅改写注释，旧 autopilot 交接前缀在 resume 时按普通指令处理。
- 测试 fixture 仅删除 `ap.json` 写入，`work/.opencoder` 目录创建保留
  （`runner_dispatch`、`dag_codex_runc` 等直接写 `work/.opencoder/config.json`；
  web fixture 的 `workdir` 本身也由该行隐式创建）。

改动面：`crates/session/src/autopilot/*`（整目录删除）、`crates/core/src/config/{runtime/autopilot.rs,domain.rs,merge.rs,lib.rs}`、
`crates/store/src/{types.rs,turso_store/{bootstrap/schema/{parts/part_1.rs,migrations.rs},conversation/{chat_tables.rs,sessions/parts/part_{1,2}.rs}}}`、
`crates/tui/src/{ap_menu/*,app_loop_ap*.rs,model_menu/*,chat.rs,command.rs,worker.rs}`、
`crates/web/src/{api_meta.rs,api.rs,cmd.rs,handle/drain.rs,handle_questions.rs,lib.rs}`、
`crates/web/spa/src/{chat.jsx,commandMenu.js}` 及对应测试、`scripts/{e2e,acceptance,platform}` 中的 autopilot 场景；
其余 crate（agent/control/node/local/project/todos/ctl/worker 及根 `tests/`）为
`SessionMeta`/快照构造处移除 `autopilot_mode` 字段与 fixture 去 `ap.json` 的机械改动。
不触及规则 04/06/07 契约；无 Ontology 能力登记项，不触发同步；无凭证。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 移除后旧 `autopilot` 键被吞掉、不外露 | `merge_into_hard_cuts_domain_keys_from_config_json`、`legacy_domain_keys_table` | `crates/core/src/config/merge.rs` |
| `ap.json` 退出域文件表 | `DOMAIN_FILES` 用例（`domain.rs` 测试模块） | `crates/core/src/config/domain.rs` |
| v40 迁移删除遗留列；v10 旧库升级保行且不再建列 | `schema_migration_v39_to_v40_drops_removed_autopilot_column`、`schema_migration_v10_to_latest_preserves_rows_without_removed_column` | `crates/store/tests/store_migrations/sessions.rs` |
| `SessionEvent` 变体数收敛 | `replay_kind_matches_live_kind_for_all_variants`（20）、`from_sse_roundtrips_all_variants`（24 kinds） | `crates/web/tests/replay_fidelity.rs`、`crates/session/src/runner/event/tests.rs` |
| 标题路由不再附带 verify 轮 | `title_routes_to_responses_small_model_with_usable_output_budget` | `crates/session/tests/responses/auxiliary.rs` |
| SPA 命令面板不再含 `/ap`（九个固定条目） | `the nine fixed slash entries` 等 | `crates/web/spa/src/commandMenu.test.js` |
| SPA 对话操作区无 autopilot 按钮/弹窗 | 全文件 5 用例 | `crates/web/spa/src/chat/chatActions.dom.test.jsx` |
| SPA 聊天面板 DOM 回归 | 全文件 13 用例 | `crates/web/spa/src/chat.dom.test.jsx` |
| 历史 `autopilot` 帧回放渲染臂保留 | `autopilot renders phase + iteration` | `crates/web/spa/src/reduce.test.js` |
| worker fixture 仍建 `work/.opencoder` | `registered_runner_is_rejected_by_dag_definition_and_inline_dispatch` | `crates/worker/tests/runner_dispatch.rs` |
| web fixture 仍创建 `workdir` | `web_agent_snapshot`（10）、`agent_resources`（3） | `crates/web/tests/{web_agent_snapshot.rs,agent_resources/}` |

删除的测试（随功能下线）：`crates/core/tests/config_autopilot_contract.rs`（整文件）、
`config_contract.rs::autopilot_config_roundtrips_through_save`、
`crates/session/src/autopilot/tests.rs`、`crates/session/tests/{autopilot*,resume_ap_mode}.rs`、
TUI `app_loop_ap_outcome_tests.rs`、`validate_rejects_ap_max_iter_u32_overflow`。

- 定向回归（仅本次改动涉及的 crate 与文件；未运行全量测试）：
  - Clippy：`cargo clippy -p opencoder-core -p opencoder-session -p opencoder-tui -p opencoder-web -p opencoder-local -p opencoder-node -p opencoder-worker -p opencoder-project -p opencoder-cli -p opencoder-dag-runtime -p opencoder-agent --all-targets` → 零警告；store/ctl/todos 见下方「提交树复核」
  - `cargo test -p opencoder-core` → 全部 target 通过（456 passed / 0 failed）
  - `cargo test -p opencoder-session` → 全部 target 通过（lib 477 passed）
  - `cargo test -p opencoder-tui`、`-p opencoder-local`、`-p opencoder-node`、`-p opencoder-agent` → 全部 target 通过
  - `cargo test -p opencoder-web` / `-p opencoder-project` / `-p opencoder-dag-runtime` / `-p opencoder-cli` → 通过，例外见下
  - worker 全量（52 targets，为容器 e2e 闭合而跑）：一轮 295 passed / 10 failed（8 targets）；修复 fixture 后 `runner_dispatch` 复跑 1 passed（计 296 / 9，7 targets），其余失败全部归因见下
  - 镜像前置：`scripts/prepare-dag-rootfs.sh target/debug/dag-rootfs`（本地、无网络）成功后，此前因镜像过期失败的 7 个 target 复跑全绿：dag-runtime `run_loop` 14 / `dynamic` 14 / `sandbox_environment` 2、web `dag_e2e_flow` 2、worker lib 160、project lib 33 + `executor_team_dag_brain` 5
  - SPA：`npx vitest run src/commandMenu.test.js`（19 passed）、`src/chat/chatActions.dom.test.jsx`（5 passed）、`src/chat.dom.test.jsx`（13 passed）、`src/reduce.test.js`（46 passed）
- 提交树复核（在干净 HEAD worktree 上应用暂存 diff，`-j 12`）：
  - `cargo check --workspace --all-targets` → 通过（0 warning）
  - `cargo clippy -p opencoder-store -p opencoder-session -p opencoder-tui -p opencoder-cli -p opencoder-todos --all-targets -- -D warnings` → 通过（0 警告）
  - `cargo test -p opencoder-store --no-fail-fast` → 384 passed / 4 failed（失败项全部归因如下，均非本次改动引入）
  - `cargo test -p opencoder-session --no-fail-fast` → 859 passed / 0 failed
  - `cargo test -p opencoder-tui --no-fail-fast` → 1957 passed / 0 failed
- 失败项归因（均非本次改动引入；复核中发现的 fixture 回归已修复并记入上表）：
  - 并行负载抖动（隔离复跑通过）：`harness_settings_queue` 全 target 5 passed（5.34s）、`harness_matrix` 1 passed（33.16s）、`dag_live_logs` 1 passed（4.73s）；全量并行时表现为 `Codex start count` / `did not settle` 超时。
  - 环境前置：`operator_initialization_recovery`（`execution preflight: agent codexops unavailable`，宿主无 codex 二进制）；`dag_codex_runc`（`runc_fixture::runner` 相对测试二进制目录解析 `examples/agent-step-runner`，落在 `target/debug/build/.../out/` 下报 missing runner，该文件本次未改）；`dag_image_recovery`（`cp -al` 自 `target/debug/dag-rootfs` 到 `/tmp` 跨文件系统硬链接失败）；`maintenance_dialogs_clear::execution_profile_discovery_is_read_only_and_does_not_expose_settings`（宿主存在 operator `config.json` 泄漏进 fixture，`maintenance.rs:19-24`）。
  - 既有失败（相关代码路径不在本次改动内，上一轮干净 HEAD 对照复跑同样失败）：`web sse_fingerprint_ttl::seen_fingerprints_expire_at_first_forwarded_done`（期望 2 得 1；去重路径 `api_events.rs`/`sse_replay.rs` 仅格式化差异）、`cli server_local::dag_definitions_crud_roundtrip`（`catalog_guard` 引用检查 `No such file or directory`，`crates/control/src/catalog_guard/` 本次未改）。
  - 环境前置（提交树复核）：`catalog_maintenance::{legacy_v32_catalog_migrates_and_restores_old_writes_without_auth_changes,v31_migration_restarts_and_restores_old_project_writes_without_auth_changes}`（用例经 python `sqlite3` 构造旧库，宿主 python 缺 `_sqlite3` 模块）。
  - 既有失败（提交树复核，干净 HEAD `77f8150c7` 对照复跑同样报错）：`list_activity_order::existing_schema_reopens_with_activity_pagination_and_unchanged_history`（重开库后 `no such index: idx_sessions_activity`；本次仅改该用例的 watermark 断言与 fixture 字段）、`brain_layered_v4::reporting::legacy_guidance_queue_migrates_to_indexed_heads_and_ack_is_atomic`（`no such column: eligible`，reporting 模块本次未改）。
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。
