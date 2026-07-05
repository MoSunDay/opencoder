Commit: (working-tree, pre-initial-commit)

# store 模块

## 职责
会话历史的唯一持久化层。封装 sessions / messages / session_inputs / session_events 四类数据的 CRUD，对外暴露 `Store` trait，对内提供 libsql（本地嵌入 + WAL）实现。

## 边界与非目标
- 不做任何 LLM / agent 逻辑（纯存储）。
- 不直接被 TUI/CLI 调用业务逻辑——上层经 `Arc<dyn Store>` 依赖。
- 非目标：远程/turso 复制（当前仅 local embedded）。

## 关键抽象
- `Store` trait（`src/store.rs`）：async_trait，dyn 兼容。这是「切换其它 Rust SQLite 实现」的唯一接缝——换后端只需新实现 trait，上层零改动。
- `LibsqlStore`（`src/libsql_store/mod.rs`）：持有**单个** Connection（`db.connect()` 一次），每个 op clone。libsql 的 `:memory:` 每次 connect 返回独立空库，故必须缓存连接共享——这是正确性关键。
- schema（`src/libsql_store/schema.rs`）：5 表 + 3 索引 + `schema_version`。bootstrap 幂等；`PRAGMA journal_mode=WAL` 等 per-connection 应用（注意该 pragma 返回行，必须用 `query`+drain，`execute` 会报 "Execute returned rows"）。
- 类型（`src/types.rs`）：`SessionMeta`/`SessionPatch`/`SessionFilter`/`SessionListItem`/`Delivery{Steer,Queue}`/`SessionInput`/`SessionEventRecord`/`EventKind`。

## 主流程
- 写消息：`append_message` / `append_messages`（事务，all-or-nothing）。
- 输入提升：`admit_input`（计算单调 admitted_seq）→ `pending_inputs` 查询 → `promote_inputs`（按 admitted_seq 截止批量标记）/ `claim_next_queue`（原子返回+标记单条 queue，供 drain idle 消费）。
- 事件回放：`append_event` → `events_after(seq)` 供 SSE replay。
- 迁移：`src/import.rs::import_jsonl_dir` 把旧 `<id>.jsonl` 一次性导入（幂等，已存在的 session 跳过）。

## 依赖与接口
- 依赖：libsql 0.9.30（锁定 0.9 系列）、opencode-core（Message 类型）、async-trait。
- 被依赖：session（resume/drain/record）、web（AppState.store）、cli（session 子命令）。

## 相关模块
- [agents/session](../session/index.md) — 通过 Store 持久化与 resume。
- [agents/web](../web/index.md) — 通过 Store 做 prompt admit 与事件回放。

## 代表性锚点
- WAL 并发读写契约：`tests/store_integration.rs::concurrent_readers_while_writer`
- 崩溃恢复：`tests/store_integration.rs::wal_crash_recovery`
- 事务回滚：`tests/store_integration.rs::transaction_rollback_on_partial_failure`
- 性能门槛：`tests/store_perf.rs`（0.031ms/append、2.4ms/load1000、0.95ms/list200）
