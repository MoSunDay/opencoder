Commit: d146e517f8b31ba3f8e5a1e493e0450d0c50d624

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
- `src/schedule_types.rs` — `ScheduleRunRecord` 与 `SCHEDULE_RUN_FIRED/MISSED/ERROR` 常量；`src/libsql_store/schedule.rs` — `schedule_runs` 台账（schema v26，PK `(schedule_id, scheduled_for_ms)`，error 重试 `INSERT OR REPLACE` 收敛）
- `src/fleet/` — 节点容量、运行时归属与派发回执（`handoff/`）；`FleetStore::execution_names` 按 id 批量查派发时名称快照（单条 SQL，无 N+1）

## 接缝
- `Arc<dyn Store>`：session/web/todos/control 等全部经此持久化。
