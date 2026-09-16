Commit: e797e184412ac6df34cd5e8189634e1361945362

# store 模块

`Store` trait + libsql（WAL）持久化层。细节以代码为准。

## 索引
- `src/lib.rs` — `Store` trait
- `src/libsql/` — libsql 实现（WAL）
- `src/fleet/` — 节点容量、运行时归属与派发回执（`handoff/`）；`FleetStore::execution_names` 按 id 批量查派发时名称快照（单条 SQL，无 N+1）

## 接缝
- `Arc<dyn Store>`：session/web/todos/control 等全部经此持久化。
