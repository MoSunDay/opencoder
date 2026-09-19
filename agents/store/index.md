Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# store 模块

`Store` trait + libsql（WAL）持久化层。细节以代码为准。

## 索引
- `src/lib.rs` — `Store` trait
- `src/libsql_store/` — libsql 实现（WAL）
- `src/libsql_store/node_tasks.rs` — 节点任务（会话绑定）读写；
  `clear_finished_sessions` 单事务清扫单节点终态会话（`clear_node_dialogs`，仅 web 开发面使用）
- `Store::delete_sessions`（`src/libsql_store/sessions.rs`）批删会话（FK 级联 messages）；
  `FleetStore::delete_terminal_indexes`（`src/fleet/records.rs`）按 node+kind+可删状态(idle|done|error|cancelled)
  删 `execution_index` 连带 assignments/receipts——生产一键清空走这两条（见 control 中继）
- `src/schedule_types.rs` — `ScheduleRunRecord` + `SCHEDULE_RUN_FIRED/MISSED/ERROR` 常量，`ScheduleDefRecord`（`job` JSON + created_at/updated_at）；`src/libsql_store/schedule.rs` — `schedule_runs` 台账（schema v26 起，PK `(schedule_id, scheduled_for_ms)`，error 重试 `INSERT OR REPLACE` 收敛）+ `schedules` 定义表（schema v27：PK id、`job` JSON、时间戳；upsert 保留 created_at）——**调度定义事实源**，`schedules.json` 降级为一次性 seed；删除定义不级联台账（无 FK，审计长存）
- `src/fleet/` — 节点容量、运行时归属与派发回执（`handoff/`）；`FleetStore::execution_names` 按 id 批量查派发时名称快照（单条 SQL，无 N+1）
- `src/libsql_store/brain_scheduler.rs` — v3 大脑 scheduler run/operation/event 投影；generation 栅栏、终态冻结、终态事件去重和按 run/seq 分页均在 Store 事务内完成，事件不存执行正文。

## 接缝
- `Arc<dyn Store>`：session/web/todos/control 等全部经此持久化。
