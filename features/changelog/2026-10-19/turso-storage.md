Commit: bb06945c1623a3e664e743e3f2a2bcbfd2cf2085

# Turso 存储与本机引擎切换

Store、项目存储、Fleet、TsRegistry 和 Ontology 改用 Turso 0.8.3 的异步 Rust API，保留现有上层接口及 schema 39。每个文件复用独立读连接和单写连接；完整操作持有同一连接，读操作使用快照。数据库任务在独立执行线程运行，应用请求取消后，已接收的写入仍完成提交或回滚。持久化使用 WAL 和 `synchronous=FULL`，不启用 MVCC。

会话列表沿活跃时间索引筛选和分页，再读取消息预览；消息、事件和队列操作复用预编译 SQL。混合负载工具使用同一份数据库快照，对比旧引擎默认持久化、旧引擎 FULL 和 Turso FULL，分别记录吞吐、延迟、内存及应用运行时等待。

开发和测试构建单独优化 Turso 的 Rust 查询引擎，避免未优化的数据库执行耗尽应用心跳时限；应用自身仍保留调试构建设置。

发布数据格式升级到 11，并增加存储引擎标记，拒绝两个引擎同时打开同一份在线数据库。本机维护先关闭接收并等待任务完成，再停止写入、备份和切换程序；所有登记的历史 Runtime 保留 ID、数据、执行归属与资源快照，程序和镜像使用独立关联。开放写入前仅回退程序和路由，开放后使用修正版发布；引擎切换流程不恢复旧数据库。

在线维护查询、事务与备份通过原生 CLI 使用 Turso。SQLite 仅用于旧引擎或已停止的数据库和完成的离线备份。原生备份校验后才发布目标文件，保留已有鉴权数据。

| 功能 | 定向验证 |
| --- | --- |
| 取消后完成已接收写入、快照与失败回滚 | `opencoder-database` 的 `concurrency`、`cancelled_caller_does_not_remove_an_admitted_queued_write`、`cancellation_while_executor_is_full_keeps_owned_work` |
| 多进程 WAL、在线备份、停止后 checkpoint | `opencoder-database` 的 `processes`、`maintenance` |
| 原生维护事务、只读限制、BLOB 与路径转义 | `rolling_tests/test_native_database.py` |
| Store/ProjectStore SQL 兼容 | `store_integration`、`store_migrations`、`project_store` 及相应分页、队列、节点、调度用例 |
| 现有 schema 的索引补齐及分页、历史保留 | `existing_schema_reopens_with_activity_pagination_and_unchanged_history`、`list_activity_order` |
| 单项延迟与混合吞吐 | `store_perf`、`examples/storage_mixed.rs` |
| 大批量节点上报期间的心跳与取消回复 | `slow_full_report_commit_preserves_heartbeat_and_owned_cancel_reply` |
| Ontology 事务与查询 | Ontology 单元测试及存储、实体查找、图查询集成测试 |
| 本机切换与程序回退保留数据库 | `maintenance_tests/engine/test_engine.py` |
| 历史 Runtime 程序关联保留旧镜像与配置 | `maintenance_tests/engine/test_programs.py` |
| 历史数据库备份容量 | `test_engine_cutover_budgets_retained_runtime_databases_and_wal` |

关联说明：[Store](../../../agents/store/index.md)、[Ontology](../../../agents/ontology/index.md)、[节点平台](../../agent-platform/index.md)、[发布流程](../../../docs/smooth-release.md)。

测试使用临时数据；具体发布与性能结果由对应的执行回执记录。
