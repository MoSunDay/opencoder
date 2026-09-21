Commit: 40a688a77bfdbedc3f30f9f6b1e3a1ba67244d68

# store 模块

`Store` trait + libsql（WAL）持久化层。细节以代码为准。
接缝：`Arc<dyn Store>`，session/web/todos/control 等全部经此持久化。

## 索引
- `src/lib.rs` — `Store` trait
- `src/libsql_store/` — libsql 实现（WAL）
- `src/libsql_store/sessions.rs` — 会话批删（FK 级联）；`node_tasks.rs` — 节点任务与终态清扫
- `src/types.rs` — `SessionMeta.kind` 泳道标签（schema v28 起 `sessions.kind TEXT`；创建时定值：`operator`/`agent`/`team`/`dag`/`todos`/`project`/`brain`，存量行为 NULL）
- `src/libsql_store/sessions.rs` 泳道栅栏 — `SessionFilter.kind=None` 的默认清单排除 `kind='operator'`（`s.kind IS NULL OR s.kind <> 'operator'`），精确泳道用 `s.kind = ?`；存量 NULL 行仍走 id 前缀/标题回退
- `src/schedule_types.rs`、`src/libsql_store/schedule.rs` — 调度台账与定义表（schema v26/v27）
- `src/fleet/` — 节点容量/归属/派发回执（`handoff/`），容量领取在 `handoff/capacity.rs`
- `src/fleet/records.rs` — 终态执行索引批删
- `src/libsql_store/brain_scheduler.rs` — v3 调度 run/operation/event 投影
- `src/libsql_store/brain_layered.rs` + `brain_layered/schema.rs` — v4 分层画布 run/operation/event 投影（additive 建表，不推动 `SCHEMA_VERSION`；`schema_watermark()` 仅供断言，当前值为 28）
