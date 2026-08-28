Commit: (working-tree, sandbox 模式替换 plan/act 双模式)

# store 模块

## 职责

会话与 TODO 工作流的唯一持久化层。`Store` trait 隔离上层运行时与 libsql 实现，保存 Session、Message、Input、Event、Subagent 关系、TODO Workflow projection 和 append-only TODO Event。

## 边界

- 不执行 LLM、agent、TUI 或工作流决策。
- 上层只依赖 `Arc<dyn Store>`，不直接依赖 libsql 查询。
- 当前后端是本地 embedded libsql + WAL；远程复制不是该模块能力。
- 删除 Session 或数据库数据必须由显式上层操作触发。

## 关键抽象

- `Store`（`src/store.rs`）：dyn-compatible async 持久化接口。默认方法只用于后端兼容；libsql 实现完整支持 todos。
- `LibsqlStore`（`src/libsql_store/mod.rs`）：缓存单个 Connection，并以 async Mutex 串行触碰同步 SQLite FFI，避免并发 worker 阻塞。
- `run_tx`（`src/libsql_store/tx.rs`）：显式 BEGIN/COMMIT/ROLLBACK，避免 async 取消时 `libsql::Transaction::Drop` panic。
- Session 类型（`src/types.rs`）：`SessionMeta`、`SessionPatch`、Input/Event/Subagent records；`task_type` 区分 parent、subagent、todo_workflow 和 todo。
- TODO 类型（`src/todo_types.rs`）：`TodoWorkflowRecord`、`TodoItemRecord`、`TodoEventRecord` 和列表摘要。

## Schema 与一致性

schema 当前为 v10：

- Session 面：`sessions`、`messages`、`session_inputs`、`session_events`、`subagent_tasks` 及 ts registry 相关结构。
- TODO 面：`todo_workflows` 保存 spec/state/generation；`todo_items` 保存每项 projection；`todo_events` 保存有序不可变 transition。
- v9 migration 从既有 v8 数据库新增 TODO 表和索引，不修改既有 Session 数据。
- v10 migration 给 `session_inputs` 加 `recorded` 消费标记（NOT NULL DEFAULT 0）：promote（含再提升）时重置 0，消费后 `mark_inputs_recorded` 置 1；promoted-but-unrecorded 孤儿行（崩溃/硬中止残留）由 `recover_orphan_inputs` 翻回 pending；迁移落地时既有 promoted 行一次性回填 recorded=1。
- v10 migration 给 sessions 加 plan 阶段落库两列（`plan_snapshot TEXT`、`plan_input_count INTEGER NOT NULL DEFAULT 0`）——plan/act 双模式删除后运行时已不再读写这两列，保留仅为兼容旧库 schema（读路径 `normalize_agent` 把存量 `agent='plan'` 归一为 `act`，原始行不重写）。
- `commit_todo_transition` 在单事务内更新 workflow、替换 TODO projection 并追加 event；workflow update 带 expected generation，陈旧父进程不能覆盖 interrupt 或其他 writer。
- Foreign key 将 parent/active TODO Session 关联到 `sessions`，因此 dispatch 先创建 Session，再提交 active reference。
- 消息批量写按 200 条分块；WAL 使用 30 秒 busy timeout 和被动 checkpoint。

## 主流程

- Session：create/get/list/update/delete；append messages；admit/promote/claim inputs + 落账与孤儿回收（`mark_inputs_recorded` 幂等标记已消费、`recover_orphan_inputs` 把 promoted 未落账行翻回 pending）；append/replay events；记录 subagent 生命周期。
- Resume：上层读取 SessionMeta、压缩摘要和保留消息，Store 不推断 agent 行为。
- Bundle：`src/bundle.rs` 递归导出/导入 Session 与 subagent 树，不包含 Config 或 API key。
- TODO：create workflow → 按 generation 原子 commit projection/event → list/load/events-after；interrupt、resume 和 debug projection 都以这些数据为源。
- Migration：bootstrap 幂等创建当前表，再按 `schema_version` 增量迁移；旧数据库保持可打开。

## 依赖与接口

- 依赖 libsql 0.9.x、opencoder-core message 类型和 async-trait。
- 被 session、web、cli、tui 和 [todos](../todos/index.md) 依赖。
- 用户能力见 [TODO 工作流](../../features/todos/index.md) 与 [会话 CLI](../cli/index.md)。

## 代表性验证

- `tests/store_integration.rs`：WAL 并发、事务回滚、取消安全和崩溃恢复。
- `tests/todos_workflow.rs`：TODO 投影+事件原子提交、generation 冲突、v8→v9 migration。
- `tests/legacy_agent_normalization.rs`：存量 `agent='plan'` 行在全部读路径（get/list/fork 等）归一为 `act`，原始行不被重写。
- `tests/store_perf.rs`：持久化性能门槛。
- `src/bundle.rs` 相关测试：Session 树导入导出与幂等性。
