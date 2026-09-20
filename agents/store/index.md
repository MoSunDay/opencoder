Commit: 522cd534c427cec2a55d51ea6955c0774a90a63b

# store 模块

`Store` trait + libsql（WAL）持久化层。细节以代码为准。
接缝：`Arc<dyn Store>`，session/web/todos/control 等全部经此持久化。

## 索引
- `src/lib.rs` — `Store` trait
- `src/libsql_store/` — libsql 实现（WAL）
- `src/libsql_store/sessions.rs` — 会话批删（FK 级联）；`node_tasks.rs` — 节点任务与终态清扫
- `src/schedule_types.rs`、`src/libsql_store/schedule.rs` — 调度台账与定义表（schema v26/v27）
- `src/fleet/` — 节点容量/归属/派发回执（`handoff/`），容量领取在 `handoff/capacity.rs`
- `src/fleet/records.rs` — 终态执行索引批删
- `src/libsql_store/brain_scheduler.rs` — v3 调度 run/operation/event 投影
