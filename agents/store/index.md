Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# store 模块

全部业务持久化：Store trait + libsql 默认后端。

## 关键路径
- `src/store.rs` — `Store` trait（dyn-compatible），上层只依赖 `Arc<dyn Store>`。
- `src/libsql_store/mod.rs` — `LibsqlStore` 单 Connection + async Mutex 串行访问。
- `src/libsql_store/schema.rs` — `SCHEMA_VERSION = 23`；embedded libsql + WAL。
- `src/libsql_store/schema.rs` — PRAGMA 顺序：synchronous=NORMAL 必须先于 journal_mode=WAL；busy_timeout 30s。
- `src/libsql_store/tx.rs` — `run_tx` 显式事务；写事务一律 BEGIN IMMEDIATE。
- `src/libsql_store/messages.rs` — 批量写按 `BATCH_CHUNK=200` 分块。
- `src/libsql_store/sessions.rs` — `harness_runtime`/`set_message_usage` 私有补写接口。
- `src/libsql_store/todos.rs` — `commit_todo_transition` 单事务 + expected generation。
- `src/libsql_store/project_runs.rs` — run 文本单字段 64 KiB 上限、整页 512 KiB 预算（`src/project_types.rs`）。
- `src/libsql_store/{project.rs,project_runs.rs,schema/project_relations.rs}` — project 三表 + 运行留痕。
- `src/fleet/` — `FleetStore` 独立 control.db：节点 + 五字段 execution_index。
- `src/sql_store/` — feature-gate `mysql`/`starrocks` 后端，仅覆盖 project 面。
- `src/project_factory.rs` — `open_project_store` 返回 `Arc<dyn ProjectStore>`。
- `src/project_executor_spec.rs` — `TeamSpec`/`BrainRoutes`/`validate_spec` 纯类型。
- `src/bundle.rs` — Session 树二进制导出/导入。
- `src/{types,todo_types,team_types,brain_types}.rs` — 各面记录类型。
- `src/ts_registry.rs` — tmux 会话索引 `ts.db`，不含会话内容。

## 边界
- 平台库分立：control.db / definitions.db（Server）、runtime.db（Node）。
- `team_topic_runs` created_at 首插冻结；`brain_plans.tree_json` 对 store opaque。
- StarRocks 全语句走 text 协议、无跨语句事务：跨表原子提交写前拒绝。
- 删除数据必须由显式上层操作触发。

## 相关
- [agents/session](../session/index.md) — 主要消费方。
- [agents/core](../core/index.md) — HarnessRuntime 类型来源。
- [agents/project](../project/index.md) — project 运行留痕边界。
