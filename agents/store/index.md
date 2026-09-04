Commit: (working-tree, sandbox 回退为 plan：恢复 plan/act 双模式回切，写拦截能力保留)

# store 模块

## 职责

会话与 TODO 工作流的唯一持久化层。`Store` trait 隔离上层运行时与 libsql 实现，保存 Session、Message、Input、Event、Subagent 关系、TODO Workflow projection 和 append-only TODO Event。

## 边界

- 不执行 LLM、agent、TUI 或工作流决策。
- 上层只依赖 `Arc<dyn Store>`，不直接依赖 libsql 查询。
- 默认后端是本地 embedded libsql + WAL；`storage.project` 可切 feature-gate 的 sqlx 后端（mysql/starrocks，仅覆盖 project 面，见下）。远程复制不是该模块能力。
- 删除 Session 或数据库数据必须由显式上层操作触发。

## 关键抽象

- `Store`（`src/store.rs`）：dyn-compatible async 持久化接口。默认方法只用于后端兼容；libsql 实现完整支持 todos。
- `LibsqlStore`（`src/libsql_store/mod.rs`）：缓存单个 Connection，并以 async Mutex 串行触碰同步 SQLite FFI，避免并发 worker 阻塞。
- `run_tx`（`src/libsql_store/tx.rs`）：显式 BEGIN/COMMIT/ROLLBACK，避免 async 取消时 `libsql::Transaction::Drop` panic。
- Session 类型（`src/types.rs`）：`SessionMeta`、`SessionPatch`、Input/Event/Subagent records；`task_type` 区分 parent、subagent、todo_workflow 和 todo。
- TODO 类型（`src/todo_types.rs`）：`TodoWorkflowRecord`、`TodoItemRecord`、`TodoEventRecord` 和列表摘要。
- 项目面类型与接缝（`src/project_types.rs` / `src/project.rs` / `src/project_factory.rs`）：goal→milestone→todo 三级 + `project_todo_runs` 运行留痕；`ProjectStore` trait（18 方法，独立于 `Store`）+ `open_project_store(config)` 工厂（libsql 默认 / `mysql` / `starrocks` feature 二选一）——opencoder-project 运行时持有 `Arc<dyn ProjectStore>`，会话/消息仍走 `Arc<dyn Store>`。
- 组队台账类型（`src/team_types.rs`，v17）：`TeamTopicRunRecord`——opencoder-team 话题 × node 的持久配对行（status `executing|finished`，`created_at` 首插冻结），运行时在 Store 之上，Store 只持久化（实现见 `libsql_store/team_runs.rs`）。

## Schema 与一致性

schema 随迭代推进（最新以 `src/libsql_store/schema.rs::SCHEMA_VERSION` 为准；brain 三表与 project 四表于 v15 落地，team 台账表于 v17 落地，brain 决策树计划表于 v18 落地）：

- Session 面：`sessions`、`messages`、`session_inputs`、`session_events`、`subagent_tasks` 及 ts registry 相关结构。
- TODO 面：`todo_workflows` 保存 spec/state/generation；`todo_items` 保存每项 projection；`todo_events` 保存有序不可变 transition。
- v9 migration 从既有 v8 数据库新增 TODO 表和索引，不修改既有 Session 数据。
- v10 migration 给 `session_inputs` 加 `recorded` 消费标记（NOT NULL DEFAULT 0）：promote（含再提升）时重置 0，消费后 `mark_inputs_recorded` 置 1；promoted-but-unrecorded 孤儿行（崩溃/硬中止残留）由 `recover_orphan_inputs` 翻回 pending；迁移落地时既有 promoted 行一次性回填 recorded=1。
- v10 migration 给 sessions 加 plan 阶段落库两列（`plan_snapshot TEXT`、`plan_input_count INTEGER NOT NULL DEFAULT 0`）——plan/act 双模式删除后运行时已不再读写这两列，保留仅为兼容旧库 schema（读路径 `normalize_agent` 把存量 `agent='plan'` 归一为 `act`，原始行不重写）。
- brain 三表（v15）：`brain_capabilities` / `brain_eng_inputs`（ON DELETE CASCADE，position 定序）/ `brain_vectors`（LE f32 BLOB，检索用 bundled `vector_distance_cos` + model 过滤防跨模型 dim 错配）；写入走 `create/update_brain_capability_with_vector` **单事务组合写**（capability+eng_inputs+vector 同提交/回滚，向量由 brain runtime 预嵌入后经 `BrainVectorWrite` 传入），另有逐步 `upsert_brain_vector` 供直接使用。
- project 四表（v15）：`project_goals` / `project_milestones`（FK→goals CASCADE）/ `project_todos`（FK→milestones SET NULL，`milestone_id` 可空即 backlog；`status` draft|planned|running|done|failed、`plan_md`、`active_session_id`）/ `project_todo_runs`（run 留痕：kind plan|execute、version、status running|done|failed|cancelled、output/plan 快照、session 引用）；删除走 goal→milestone→todo→run 显式顺序（libsql 事务内）。实现 `libsql_store/{project.rs, project_runs.rs}`。
- `brain_plans`（v18）：动态规划决策树持久层（id/situation/situation_digest/chat_model/tree_json/created_at + `idx_brain_plans_digest(digest,created_at)`）；`Store` trait 三方法 `save/get/latest_brain_plan_for` 默认 bail、libsql 实现，latest 按 `created_at DESC, rowid DESC` 全序稳定；tree_json 为 brain crate 的 `DecisionTree` 序列化（含分支主题向量），store 保持 opaque。v17→v18 迁移补表。
- `team_topic_runs`（v17）：opencoder-team 话题扇出的 `(topic_id, node_id)` 台账，PK(topic_id,node_id)、`node_id` FK→`nodes` ON DELETE CASCADE；`upsert` 冲突臂只刷 `status`（`created_at` 首插冻结，刷新不重启计时钟）、`finish` 全行翻 `finished`（幂等，未知 topic 0 行即成功）、`list` 按 `created_at, rowid` 定序（ULID 非单调不可排序）。`Store` trait 三方法（`upsert/finish/list_team_topic_run*`）默认 bail，libsql 完整实现；v16→v17 迁移补表（CREATE IF NOT EXISTS，索引落 post-batch）。
- `commit_todo_transition` 在单事务内更新 workflow、替换 TODO projection 并追加 event；workflow update 带 expected generation，陈旧父进程不能覆盖 interrupt 或其他 writer。
- Foreign key 将 parent/active TODO Session 关联到 `sessions`，因此 dispatch 先创建 Session，再提交 active reference。
- 消息批量写按 200 条分块；WAL 使用 30 秒 busy timeout 和被动 checkpoint。
- PRAGMA 顺序不变量：`synchronous=NORMAL` 必须先于 `journal_mode=WAL`（全新库切 WAL 做 header 初始化 fsync，落在当时的 synchronous 策略下，顺序颠倒即 FULL）；有单测锁定顺序。

## sqlx 后端（feature-gate：`mysql` / `starrocks`）

`src/sql_store/`（默认零编译，两个 feature 互斥二选一）：`ddl.rs` 方言化 DDL（StarRocks：列后 PRIMARY KEY + STRING/无 FK，MySQL：FK CASCADE 语义与 libsql 对齐）、`project_crud.rs` / `project_crud_runs.rs` CRUD。两条硬教训：

- StarRocks 缓存 prepared SELECT 会返回旧快照且 publish 异步——**全部语句走 text 协议**（`raw_sql` 内联参数）；sqlx 0.8.6 `RawSql::fetch_optional` 误委托 fetch_one 会 panic，用 `fetch_all` 再取 first 恢复 optional 形态。
- 级联删除跨语句无事务保证，顺序执行（run→todo→milestone→goal）；测试一律 `eventually()` 轮询收敛。

## 主流程

- Session：create/get/list/update/delete；append messages；admit/promote/claim inputs + 落账与孤儿回收（`mark_inputs_recorded` 幂等标记已消费、`recover_orphan_inputs` 把 promoted 未落账行翻回 pending）；append/replay events；记录 subagent 生命周期。
- Resume：上层读取 SessionMeta、压缩摘要和保留消息，Store 不推断 agent 行为。
- Bundle：`src/bundle.rs` 递归导出/导入 Session 与 subagent 树，不包含 Config 或 API key。
- TODO：create workflow → 按 generation 原子 commit projection/event → list/load/events-after；interrupt、resume 和 debug projection 都以这些数据为源。
- Migration：bootstrap 幂等创建当前表，再按 `schema_version` 增量迁移；旧数据库保持可打开。整段 bootstrap 在单个 `BEGIN IMMEDIATE` 事务内（17 条 DDL 不再各自隐式提交，失败整体回滚）。v14：`messages.display` TEXT 列（`add_column_if_absent` 守卫、可空）承载回显原文，INSERT/load/load_after 全链路读写（row index 8）；列可空故旧二进制显式列名语句不受影响，可随时回滚。

## 依赖与接口

- 依赖 libsql 0.9.x、opencoder-core message 类型和 async-trait。
- 被 session、web、cli、tui 和 [todos](../todos/index.md) 依赖。
- 用户能力见 [TODO 工作流](../../features/todos/index.md) 与 [会话 CLI](../cli/index.md)。

## 代表性验证

- `tests/schema_bootstrap.rs`：建库后 synchronous 生效值、同路径重开幂等（version 单行 + integrity_check）、并发打开。
- `tests/store_integration/`（目录目标，按职责分模块）：会话 CRUD/patch、消息往返、事务回滚、取消安全和崩溃恢复等 P0 行为契约（WAL 并发压力另见 `store_concurrency.rs`）。
- `tests/todos_workflow.rs`：TODO 投影+事件原子提交、generation 冲突、v8→v9 migration。
- `tests/project_store.rs`：project 四表 CRUD/级联/状态流转 8 例；`tests/sql_project_store.rs`：sqlx 后端（`OC_TEST_MYSQL_DSN` / `OC_TEST_STARROCKS_DSN` 环境门控，无 DSN 自动跳过），真 MySQL 8.4 / StarRocks 3.3 容器实测。
- `tests/team_runs.rs`：upsert 往返且 `created_at` 冻结、`finish` 全行翻转、节点删除级联；`tests/store_migrations.rs` 覆盖 v16→v17 建表。
- `tests/legacy_agent_normalization.rs`：interlude 存量 `agent='sandbox'` 行在全部读路径（get/list/fork 等）归一为 `plan`，原始行不被重写。
- `tests/store_perf.rs`：持久化性能门槛。
- `src/bundle.rs` 相关测试：Session 树导入导出与幂等性。
