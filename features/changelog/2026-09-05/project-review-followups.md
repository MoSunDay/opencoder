# opencoder-project：评审修复轮续（claim 回滚 / plan-execute 互斥 / 条件收敛 / CI 密级）

日期：2026-09-05 ｜ 提交：6cb5ea1

## 动机

上一评审轮（project-review-hardening）遗留 5 项 TODO：#N1 claim 后 `create_todo_run` 失败无补偿（todo 悬死 Running 且无 run 行，sweep 只扫 run 行永远不自愈）；#N2 plan-in-flight 与 execute 无互斥（plan 收尾无条件回写会把 Running 打回 Planned → 第三次 execute 可再 claim，双执行同一会话）；#N3 sweep 单进程独占 store 假设未文档化；#N4 starrocks DSN 走 `vars.`（对 repo 只读者可见，弱于 secrets）+ #N5 CI paths 漏 `Cargo.toml`/`Cargo.lock`；微瑕：panic 收敛无条件 close 会把已 Done 的 run 改写为 Failed。本轮全部清偿。

## 变更

### #N1 claim 失败补偿回滚（P1）

- `crates/store`：`ProjectStore` trait 新增 `patch_todo_when(id, when, patch, now) -> Result<bool>`——期望状态条件 CAS（`WHERE id=? AND status=?`），matched 行必然被改写（调用方约定 `patch.status ≠ when`，规避 MySQL affected-rows 歧义，注释已声明）。libsql / sqlx 双后端实现，SET 子句抽取纯函数 `todo_set_fragment` / `run_set_fragment` 与无条件版共享。
- `crates/project/src/service.rs::start_execute`：`create_todo_run` 失败时先 `patch_todo_when(Running → todo.status)` 条件回滚 claim 前状态（仍 Running 才动，防误伤），回滚自身失败只告警、原始错误照常上抛——悬死 Running 无 run 行的形态自此消除。

### #N2 plan/execute 双向互斥（P1）

- 正向（主窗口）：`start_execute` 在 plan_md 检查后新增 running Plan run 检查（`list_running_todo_runs` 过滤同 todo + kind=Plan），存在即 bail "todo plan generation is in progress"。
- 残余竞态（检查→claim 毫秒级窗口）：`plan_gen.rs` 新增 `commit_plan_output`——plan 收尾回写改为「重读 todo + 按观察态条件 CAS 落 Planned+plan_md」；todo 已被 execute claim（Running）则丢弃回写（方案仍留痕 run 行 output_md），Running 标签不再被打回，双执行路径关死。反向（执行中不可重 plan）由既有 status 检查保证。

### #N3 单进程假设文档化（P2）

- `agents/project/index.md` 崩溃兜底 bullet：显式声明 sweep 以本进程内存注册表判驱动存亡的**单进程独占 store 假设**——libsql 本地嵌入天然满足；mysql/starrocks 共享 DSN 多进程会误收敛活跃 run（真实 driver 终态写回可自愈，仅留 failed 噪声），多进程化前需引入持有者标记。

### #N4/#N5 CI 修正（P2）

- `.github/workflows/project-sql-tests.yml`：starrocks job 改「布尔 repo 变量 `OC_TEST_STARROCKS_ENABLED` 作 `if` 门（job 级 `if` 可用 `vars.` 不可用 `secrets.`）+ DSN 值取 `secrets.OC_TEST_STARROCKS_DSN`」——DSN 不再对 repo 只读者可见；secret 空值时测试层空 DSN 检查干净跳过。push/pull_request 双 paths 过滤补 `Cargo.toml` + `Cargo.lock`（依赖变更自此触发门禁）。

### 微瑕：panic/清扫收敛改条件 CAS

- `plan_gen.rs` 新增 `close_run_if_running`（仅 run 仍 Running 才写终态，返回是否赢得 CAS）；`recover.rs` 的 `converge_panicked_run` 与 `sweep_stale_runs` 均改走之——run 已终态（驱动在 close 与 todo 回写之间 panic）不再改写标签/输出；todo 侧改 `fail_todo_if_running`（Running→Failed 条件 CAS），run 已 Done 而 todo 仍 Running 的形态仍被补收敛不悬死；sweep 只对 CAS 赢家计数与回写 todo（并发收敛不双计）。

## 测试清单

- `crates/store`（`--test project_store` 10 passed）：新增 `conditional_patch_cas_applies_only_in_expected_state`（错误期望态 false 不盖戳、正确期望态 true、claim 回滚形状、missing id、plan 回写竞态不打花、run CAS 赢/输/终态重放）。`sql_project_store.rs` 契约扩展两方法断言（DSN 未设本地 env-skip，mysql CI 实跑）。
- `crates/project` unit（15 passed，+7）：`plan_gen` 新增 `commit_plan_output_skips_running_todo` / `commit_plan_output_writes_planned_todo` / `commit_plan_output_tolerates_missing_todo`；`service.rs` 新增 `start_execute_rejects_while_plan_run_in_flight`、`panic_convergence_keeps_terminal_run_label`、`panic_convergence_after_run_done_still_fails_stuck_running_todo`。
- `crates/project` 集成（7 passed，+1）：`tests/plan_and_execute.rs` 新增 `create_run_failure_rolls_back_claim`（`CreateRunFailingStore` 委托包装仅 `create_todo_run` 注入失败 → start_execute Err + todo 回 Planned + 无 execute run 行）。
- CI workflow：YAML 解析校验 + `vars.OC_TEST_STARROCKS_DSN` 残留 grep = 0。

## 回归门禁（全绿）

- `cargo test --workspace`（按 crate 分块全量执行，本环境长驻进程会被回收故分块）：19 crates / **302 suites / 4426 passed / 0 failed**（上轮 4418 + 本轮新增 8：store 1 + project 7）。
- `cargo clippy --workspace --all-targets -- -D warnings`：0 警告（另 store `--features mysql,starrocks` 单独过）；`cargo fmt --all -- --check`：0 差异。
- 行数门禁：迭代文件最大 service.rs 616 行 ≤ 800；新增文件 ≤400。
