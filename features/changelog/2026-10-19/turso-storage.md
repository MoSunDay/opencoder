Commit: ae321b750ec55df92bfe4aedd48661013fad990d

# Turso 存储与本机引擎切换

Store、项目存储、Fleet、TsRegistry 和 Ontology 改用 Turso 0.8.3 的异步 Rust API，保留现有上层接口及 schema 39。每个文件复用独立读连接和单写连接；完整操作持有同一连接，读操作使用快照。数据库任务在独立执行线程运行，应用请求取消后，已接收的写入仍完成提交或回滚。持久化使用 WAL 和 `synchronous=FULL`，不启用 MVCC。

会话列表沿活跃时间索引筛选和分页，再读取消息预览；消息、事件和队列操作复用预编译 SQL。混合负载工具使用同一份数据库快照，对比旧引擎默认持久化、旧引擎 FULL 和 Turso FULL，分别记录吞吐、延迟、内存及应用运行时等待。

开发和测试构建单独优化 Turso 的 Rust 查询引擎，避免未优化的数据库执行耗尽应用心跳时限；应用自身仍保留调试构建设置。

Fleet 从 JSON 清单关联历史任务时，显式将 ID 转为文本，让 Turso 使用主键索引，避免逐条扫描全部历史归属。归属冲突校验与批量写入仍在原有连接和事务内执行；已有归属保持只读，新增归属仍原子提交。

发布数据格式升级到 11，并增加存储引擎标记，拒绝两个引擎同时打开同一份在线数据库。本机维护先关闭接收并等待任务完成，再停止写入、备份和切换程序；所有登记的历史 Runtime 保留 ID、数据、执行归属与资源快照，程序和镜像使用独立关联。开放写入前仅回退程序和路由，开放后使用修正版发布；引擎切换流程不恢复旧数据库。

历史 Runtime 中已废弃的 DAG 配置字段只从新程序的配置副本中移除，原配置及资源保留。关闭接收前，候选程序逐个检查这些配置副本，避免历史 Runtime 唤醒时才发现配置不兼容。

历史数据目录使用挂载别名时，新程序优先通过已确认指向同一目录的统一路径准备镜像。跨挂载无法建立硬链接时复制文件，并保留镜像内部的硬链接关系；其他文件权限或写入错误继续中止发布，旧镜像保持不变。
准备历史镜像前，按每个 Runtime 数据目录所在的磁盘核算完整复制所需空间，不依赖硬链接节省容量。
镜像内程序的版本检查在独立挂载空间中提供临时 `/proc`，与原生执行时的环境一致；检查退出后挂载随之释放。
已存在的原生资源挂载通过导出源、端口、只读及缓存选项校验后继续复用，保留永久存储的绑定挂载和已有文件句柄，不因服务单元文本不同而重新挂载。
开放写入前回退程序时，已有程序目录原位恢复内容，保留绑定挂载及目录 inode；数据库继续保留当前内容。
恢复独立资源服务时使用原发布包中的兼容程序，避免更早的资源程序拒绝已支持的数据库版本。已登记的候选 Runtime 先启动兼容程序，再恢复 Host；再次切换前将数据库附属日志和共享内存文件的属主恢复为数据库服务用户。

在线维护查询、事务与备份通过原生 CLI 使用 Turso。SQLite 仅用于旧引擎或已停止的数据库和完成的离线备份。原生备份校验后才发布目标文件，保留已有鉴权数据。

外部资源文件的引用检查保留 SQLite 编码兼容：通过 rusqlite 只读检查临时副本，支持 UTF-16 和经校验的 WAL。它在资源检查的阻塞线程中执行，SQLite 连接只打开临时副本；应用存储和维护访问继续使用 Turso。

| 功能 | 定向验证 |
| --- | --- |
| 取消后完成已接收写入、快照与失败回滚 | `opencoder-database` 的 `concurrency`、`cancelled_caller_does_not_remove_an_admitted_queued_write`、`cancellation_while_executor_is_full_keeps_owned_work` |
| 多进程 WAL、在线备份、停止后 checkpoint | `opencoder-database` 的 `processes`、`maintenance` |
| 原生维护事务、只读限制、BLOB 与路径转义 | `rolling_tests/test_native_database.py` |
| Store/ProjectStore SQL 兼容 | `store_integration`、`store_migrations`、`project_store` 及相应分页、队列、节点、调度用例 |
| 现有 schema 的索引补齐及分页、历史保留 | `existing_schema_reopens_with_activity_pagination_and_unchanged_history`、`list_activity_order` |
| 单项延迟与混合吞吐 | `store_perf`、`examples/storage_mixed.rs` |
| 大批量节点上报期间的心跳与取消回复 | `slow_full_report_commit_preserves_heartbeat_and_owned_cancel_reply` |
| 历史归属索引查找及两万条清单校验 | `inventory_owner_lookup_uses_primary_key_search`、`large_inventory_preserves_owners_across_activation_and_duplicate_ids` |
| Ontology 事务与查询 | Ontology 单元测试及存储、实体查找、图查询集成测试 |
| 本机切换与程序回退保留数据库 | `maintenance_tests/engine/test_engine.py` |
| 历史 Runtime 程序关联保留旧镜像与配置 | `maintenance_tests/engine/test_programs.py` |
| 跨挂载镜像复制、内部硬链接及错误保留 | `rolling_tests/test_native.py` 的 `test_retained_image_copies_across_mounts_and_preserves_internal_links`、`test_retained_image_does_not_hide_link_permission_errors` |
| 历史数据库备份容量 | `test_engine_cutover_budgets_retained_runtime_databases_and_wal` |
| 历史程序在各磁盘上的完整复制容量 | `test_retained_programs_reserve_full_copies_on_the_data_directory_disk` |
| 保留兼容的资源绑定挂载与文件句柄 | `test_compatible_active_bind_mount_keeps_its_unit_and_kernel_handles` |
| 程序目录原位恢复 | `test_directory_restore_preserves_existing_root_and_nested_inodes` |
| 恢复兼容资源程序及候选 Runtime 启动顺序 | `test_failure_before_reopening_restores_programs_without_restoring_database`、`test_recovery_starts_registered_candidate_before_restored_host` |
| 检查点附属文件权限与符号链接边界 | `maintenance_tests/engine/test_files.py` |
| 离线 SQLite 资源的编码、引用与 WAL 保留 | `catalog_guard::binary_resources` |

关联说明：[Store](../../../agents/store/index.md)、[Ontology](../../../agents/ontology/index.md)、[节点平台](../../agent-platform/index.md)、[发布流程](../../../docs/smooth-release.md)。

测试使用临时数据；具体发布与性能结果由对应的执行回执记录。
