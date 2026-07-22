Commit: (working-tree, pre-initial-commit)

# store 模块

## 职责
会话历史的唯一持久化层。封装 sessions / messages / session_inputs / session_events / subagent_tasks 五类数据的 CRUD，对外暴露 `Store` trait，对内提供 libsql（本地嵌入 + WAL）实现。

## 边界与非目标
- 不做任何 LLM / agent 逻辑（纯存储）。
- 不直接被 TUI/CLI 调用业务逻辑——上层经 `Arc<dyn Store>` 依赖。
- 非目标：远程/turso 复制（当前仅 local embedded）。

## 关键抽象
- `Store` trait（`src/store.rs`）：async_trait，dyn 兼容。这是「切换其它 Rust SQLite 实现」的唯一接缝——换后端只需新实现 trait，上层零改动。
- `LibsqlStore`（`src/libsql_store/mod.rs`）：持有**单个** Connection（`db.connect()` 一次），每个 op clone。libsql 的 `:memory:` 每次 connect 返回独立空库，故必须缓存连接共享——这是正确性关键。 另持 `db_lock: tokio::sync::Mutex<()>`，在全部 26 个 async Store 方法入口 `lock().await` 串行化——libsql 0.9.x 把同步 SQLite FFI 直接跑在 tokio worker 线程，并发 op（多 subagent flusher + run_loop）争 SQLite 内部互斥锁会饿死 runtime；async Mutex 争用时 yield（不阻塞 worker 线程），保证同一时刻至多一个 worker 触碰 FFI。
- schema（`src/libsql_store/schema.rs`）：6 表 + 5 索引 + `schema_version`（当前 v3）。bootstrap 幂等：先 CREATE TABLE IF NOT EXISTS 全表，再读已存版本做**增量迁移**（`migrate(from)`：v2 加 `session_events.sse_kind TEXT`、v3 对 `sessions` `ALTER TABLE ADD COLUMN` 加 `handoff_seq INTEGER`/`handoff_plan TEXT`/`skill TEXT`，均 nullable 故旧行仍合法）；新库（version None）已含全量 schema 故跳过迁移，仅写版本号。`PRAGMA journal_mode=WAL` 等 per-connection 应用（注意该 pragma 返回行，必须用 `query`+drain，`execute` 会报 "Execute returned rows"）。`subagent_tasks` 表记录父子 agent 关系（task_id/parent/child session_id/prompt/result/status）。
- 类型（`src/types.rs`）：`SessionMeta`/`SessionPatch`/`SessionFilter`/`SessionListItem`/`Delivery{Steer,Queue}`/`SessionInput`/`SessionEventRecord`（含 `sse_kind: Option<String>`——细粒度 SSE 事件名，replay 优先取它、`None` 时回退 `event_kind_str(coarse)`）/`EventKind`（12 变体，粗粒度，仅作 DB `type` 列与回退）/`SubagentTaskRecord`/`SubagentStatus{Running,Completed,Failed}`。Store trait 含 `create_subagent_task`/`complete_subagent_task`/`list_subagent_tasks` 三方法。`SessionMeta`/`SessionPatch` 额外暴露 v3 三列 `handoff_seq: Option<i64>`/`handoff_plan: Option<String>`/`skill: Option<String>`（plan→act 移交边界 + 技能持久化），libsql 的 INSERT/SELECT/update handler/`row_to_meta` 四处读写。

## 主流程
- session 生命周期：`create_session` / `get_session` / `list_sessions`（`SessionFilter`：workdir_hash / search / cursor 分页）/ `update_session`（`SessionPatch` 局部更新）/ `delete_session`。`clear_other_sessions(keep)` 单条 `DELETE FROM sessions WHERE id != keep` 批量清理（保留当前会话），子表经 `ON DELETE CASCADE` 外键级联删除，返回删除条数。
- 写消息：`append_message` / `append_messages`（事务，all-or-nothing）。
- 输入提升：`admit_input`（计算单调 admitted_seq）→ `pending_inputs` 查询 → `promote_inputs`（按 admitted_seq 截止批量标记）/ `claim_next_queue`（原子返回 `(seq, SessionInput)` + 标记单条 queue，供 runner drain idle 消费；seq 随 `QueueConsumed` 事件回传前端收缩镜像）。`delete_input`（带 `promoted_seq IS NULL` 守卫，不删已提升行）/ `swap_input_order`（交换两行 admitted_seq，无 UNIQUE 约束可直接交换）供 TUI 队列面板删除/重排未消费的 follow-up。
- 事件回放：`append_events(&[record])`（**批量**，单事务 all-or-nothing，返回 seq 数组；`append_event` 单条默认委托它——批量 INSERT + 单次 `SELECT seq ... LIMIT N` backfill + reverse 还原写入序）→ `events_after(seq)` 供 SSE replay。批量写是高频表面（token delta 流）的首选路径，把 O(tokens) 次 fsync 降到 O(turn)。
- 迁移：`src/import.rs::import_jsonl_dir` 把旧 `<id>.jsonl` 一次性导入（幂等，已存在的 session 跳过）。
- **二进制导出/导入**（`src/bundle.rs`）：`SessionBundle` 递归结构（meta + messages + events + inputs + subagents）。自定义 opencoder 二进制格式（magic `OPENCODR` + 版本 + payload）。`export_bundle` 递归收集父子 session 树；`import_bundle` 幂等写入（已存在则跳过）。CLI：`opencoder session export <id> -o <file>`（默认输出 `<id>.opencoder`）/ `opencoder session import <file>`（读取 `.opencoder` 二进制）。不导出 Config（含 API key）。

## 依赖与接口
- 依赖：libsql 0.9.30（锁定 0.9 系列）、opencoder-core（Message 类型）、async-trait。
- 被依赖：session（resume/drain/record）、web（AppState.store）、cli（session 子命令）。

## 相关模块
- [agents/session](../session/index.md) — 通过 Store 持久化与 resume。
- [agents/web](../web/index.md) — 通过 Store 做 prompt admit 与事件回放。

## 代表性锚点
- WAL 并发读写契约：`tests/store_integration.rs::concurrent_readers_while_writer`
- 崩溃恢复：`tests/store_integration.rs::wal_crash_recovery`
- 事务回滚：`tests/store_integration.rs::transaction_rollback_on_partial_failure`
- 性能门槛：`tests/store_perf.rs`（0.031ms/append、2.4ms/load1000、0.95ms/list200）
ation.rs::transaction_rollback_on_partial_failure`
- 性能门槛：`tests/store_perf.rs`（0.031ms/append、2.4ms/load1000、0.95ms/list200）
