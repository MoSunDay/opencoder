# opencoder-project：评审修复轮（execute CAS / panic 收敛 / plan 快照 / SPA 净化 / CI 接线）

日期：2026-09-05 ｜ 提交：(working tree)

## 动机

对 opencoder-project 模块六 Phase 迭代的逻辑评审发现 4 项瑕疵（低危，不影响主流程）与 2 项收尾欠账：`start_execute` TOCTOU、panic 无收敛且无 stale run 清扫（重启也不自愈）、execute run 行不落 plan_md 快照（文档表述失准）、SPA markdown 无净化（继承面），外加 sqlx 后端无 CI 接线、整仓绿色终验未完成。本轮全部清偿。

## 变更

### #1 start_execute TOCTOU 关死（P1）

- `crates/store/src/project.rs`：`ProjectStore` trait 新增 `claim_todo_running(id, now_ms) -> Result<bool>`（单条条件 UPDATE：`SET status='running', updated_at=? WHERE id=? AND status<>'running'`，matched 行必然被改写，无 MySQL affected-rows 歧义）与 `list_running_todo_runs() -> Result<Vec<ProjectTodoRunRecord>>`。
- libsql（`libsql_store/project_runs.rs` + `project.rs`，db_lock 临界区）与 sqlx（`sql_store/project_crud_runs.rs` + `mod.rs`，StarRocks 自动走 raw_sql 内联约定）双后端实现。
- `crates/project/src/service.rs::start_execute`：读后写 `patch_todo(Running)` 替换为 `claim_todo_running`，false 即 bail（并发双 execute 不可能同时通过）。

### #2 panic 收敛 + 机会式 stale run 清扫（P1）

- 新增 `crates/project/src/recover.rs`（112 行）：
  - `spawn_run_driver`：spawn 驱动任务 + 监控任务 await JoinHandle，`JoinError::is_panic()` 时调用收敛——run→failed("run driver panicked")、kind=Execute 的 todo→failed（plan 不占用 todo 状态，不动）、`forget_spawn`。正常路径仍由 drive 自身三态收敛。
  - `sweep_stale_runs(deps, grace_ms)`：`list_running_todo_runs` → 对「不在 spawns 注册表且 `now-started_at` 超 grace」的 run 收敛为 failed("stale run converged")，其 Running todo 同步 failed——进程重启丢失驱动的僵尸行自此可自愈。
- `service.rs`：plan/execute 两处 spawn 改走 `spawn_run_driver`；`overview()` 开头以 `STALE_RUN_GRACE_MS = 300_000` 机会式触发 sweep（镜像 `converge_lost_node_tasks` 读路径触发思路）。

### #3 execute run 落 plan_md 快照（P1）

- `service.rs::start_execute`：create_todo_run 的 `plan_md: todo.plan_md.clone()`——run 行自此记录执行起点的方案版本，历史 run 可行内追溯（此前恒 None，需翻 session transcript）。
- `agents/project/index.md`：run 行内容措辞修正（execute 启动时落快照、plan run 只留输出）+ 补 panic 收敛/stale 清扫语义与 CAS 主流程描述。

### #4 SPA markdown 净化（P2）

- `crates/web/spa/src/project/markdown.jsx`（全 SPA 唯一 `dangerouslySetInnerHTML` sink）：marked GFM 输出经 `DOMPurify.sanitize` 后再渲染；头注释由「信任域内可接受」改为「净化 + 纵深防御」。
- `package.json`：dompurify `^3.4.14` 提升为直接依赖（原为 mermaid 传递依赖，node_modules/lockfile 离线可用）；dist 重建无漂移。

### #5 sqlx 后端 CI 接线（P2）

- 新增 `.github/workflows/project-sql-tests.yml`（仓库首个 workflow）：
  - `mysql` job：mysql:8.4 service 容器（`MYSQL_DATABASE` init env 免建库——`sql_store::open` 只建表不建库），`OC_TEST_MYSQL_DSN` 跑 `--test sql_project_store`；本地已用同款容器 + 真 DSN 实跑通过。
  - `starrocks` job：opt-in（`vars.OC_TEST_STARROCKS_DSN` 存在时的 self-hosted），官方镜像需 privileged+提 ulimit，hosted job container 表达不了。
  - `lint` job：workspace 级 fmt + clippy（本轮实测全绿后按注释约定放宽到 `--all`/`--workspace`），另加 store `--features mysql,starrocks` clippy 覆盖门禁源码。

## 测试清单

- `crates/project`（9 unit + 6 集成 = 15 passed）：`service.rs` 单元新增 `panic_convergence_fails_run_execute_todo_and_forgets_spawn`、`panic_convergence_leaves_plan_todo_untouched`、`sweep_converges_only_unregistered_runs_past_grace`（过期未注册 run 收敛、宽限期内与已注册 token 不动）；`tests/plan_and_execute.rs` 新增 `execute_run_snapshots_plan_md_at_start`。
- `crates/store`（`--test project_store` 9 passed）：新增 `claim_todo_running_cas_and_running_run_listing`（Planned→claim true 盖戳、再 claim false、unknown false、list 只返回 running）。
- SPA（vitest 34 files / 331 passed，原 328 + 新增 3）：`markdown.dom.test.jsx` script 剥离（含 onerror 内联属性）、GFM 正常渲染、空文本占位。
- CI workflow：YAML 语法校验 + 引用 cargo 命令本地干跑（skip 路径 exit 0）+ mysql 容器真 DSN 实跑 0 failed。

## 回归门禁（全绿）

- `cargo test --workspace`：302 suites / **4418 passed / 0 failed**（含 sibling 已合入的 `config_providers` 修复——上轮评审 TODO#5 的阻塞项已解除）。
- `cargo clippy --workspace --all-targets`：0 警告；`cargo fmt --all -- --check`：0 差异。
- `bash scripts/check-spa-drift.sh`：no drift；行数门禁：新增文件 ≤400（recover.rs 112 / workflow 132 / 测试文件），迭代文件 ≤800（service.rs 495 为最大）。
