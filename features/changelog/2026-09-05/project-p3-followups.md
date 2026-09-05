# opencoder-project：评审遗留 P3 清偿（stale plan 自愈 / lost-driver 取消收敛 / store trait 契约文档）

日期：2026-09-05 ｜ 提交：6cb5ea1

## 动机

project-review-followups 轮评审通过（Go、无保留），遗留两项 P3：#M1 plan-in-flight 守卫把崩溃残留的 stale plan run 行也视为进行中——execute 被阻塞至 `overview()` 触发 sweep 且过 5 分钟 grace，且 cancel 对丢失令牌只返回 false 不收敛行；#M2/#M3 `ProjectStore` 的 CAS 方法跨后端语义差异（MySQL affected-rows 按 changed 计）与「空 patch 生成非法 SET」约定只在评审报告里、未落 trait 文档。本轮清偿。

## 变更

### #M1 stale plan 守卫自愈 + lost-driver 取消收敛（`crates/project`）

- `recover.rs`：抽出 `STALE_RUN_NOTE` 统一文案与 `converge_stale_run`（单条 stale run 条件收敛：run→Failed CAS，Execute 赢家补收敛 todo；sweep 与 execute 前置守卫共用，行为不变）；新增 `converge_lost_run`（cancel 丢失令牌时：run 行仍 Running → Cancelled，Execute 的 todo Running→Planned 条件回退，镜像驱动自身取消语义）+ `revert_todo_if_running`。无需 grace：单进程假设下注册表即驱动存亡真源，且 run_id 仅在令牌注册后才对外可见（start_* 返回晚于 spawn_run），无 create→注册窗口误伤；双击 cancel 的良性竞态由驱动收尾的无条件 close_run 后写覆盖，语义不受损。
- `service.rs` `start_execute`：plan-in-flight 判定以本进程注册表为准——在注册表或 grace 内的 plan 行仍拒绝（保守，兜住并发 start_plan 的毫秒级窗口）；不在注册表且超 grace 的 stale plan 行机会式收敛后放行，消灭「崩溃后必须等总览触发 sweep」的死角。
- `service.rs` `cancel`：令牌未命中时回落 `converge_lost_run`（返回 true = 实际取消：触发令牌或收敛 lost-driver 行；行缺失/已终态 false）。

### #M2/#M3 store trait 契约文档（`crates/store/src/project.rs`，纯文档）

- 头部约定新增：`patch_*` 的 SET 子句仅由 patch 的 `Some` 字段投影——全 None patch 是调用方 bug（会产出非法空 SET）。
- `patch_todo_when` / `patch_todo_run_when` 注明跨后端差异：SQLite 按 matched 行计数（同值重写报 true），MySQL 按 changed 计（同值重写报 false、与输掉 CAS 不可区分）——调用方须保证 patch.status ≠ when 且把 false 一律按「未应用」处理，两个方向均无损。

## 测试清单

- `crates/project` unit（19 passed，+4）：`start_execute_rejects_while_plan_run_in_flight` 改为注册真令牌（新语义下仅注册表命中证明「进行中」）；新增 `start_execute_blocks_unregistered_plan_run_within_grace`、`start_execute_converges_stale_plan_run_past_grace`、`cancel_converges_lost_driver_execute_run_to_cancelled`、`cancel_terminal_or_missing_run_stays_false`。
- `crates/project` 集成（8 passed，+1）：`execute_proceeds_after_stale_plan_run_is_converged`（真 MockChatClient 执行路径 + stale plan 行被收敛为 sweep 同款文案）。
- `crates/store`：`--test project_store` 10 passed（纯文档变更，契约行为不变）。

## 回归门禁（全绿）

- `cargo test`（按 crate 分块全量执行，19 crates）：**302 suites / 4431 passed / 0 failed**（上轮 4426 + 本轮 5）。
- `cargo clippy --workspace --all-targets -- -D warnings`：0 警告；store `--features mysql,starrocks` 单独通道 0 警告；`cargo fmt --all -- --check`：0 差异。
- 行数门禁：service.rs 765 ≤ 800、recover.rs 198；changelog/文档不适用。
