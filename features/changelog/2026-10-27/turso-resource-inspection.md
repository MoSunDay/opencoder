Commit: 9a89e07b15ecc379e6171ff9f3c61b058d53a799

# 资源引用检查统一使用 Turso

目录清理的二进制资源检查去除最后一个 SQLite C 依赖（rusqlite 及其 bundled libsqlite3-sys），SQLite 格式资源改由与应用存储相同的 Turso 引擎在临时副本中只读检查。检查仍先经手写校验（WAL 头部、页大小、帧身份、校验和与最终提交）再复制，随后执行 `PRAGMA quick_check`、读取 `sqlite_schema` 结构、拒绝虚拟表并保持行数与文本字节上限；连接只打开副本，源数据库、WAL 与派生索引不被写入。检查继续运行在资源扫描的阻塞线程内，异步引擎经调用方 runtime handle 驱动。

行为变化：Turso 引擎只读取 UTF-8 数据库，UTF-16 编码的资源库不再被解码，按无法核验处理并继续阻止删除（503，此前由 C 引擎解码后命中引用返回 409）；两种情况删除都被拒绝，安全语义不变。工作区不再包含 SQLite C 代码，macOS 等平台构建不再需要 libclang/cmake。

Turso 以 WAL 为默认日志模式且提交后保留 WAL 文件，测试断言改为目录快照对比，验证检查不新增、改写或删除源旁任何文件；引擎自身不维护共享内存 WAL 索引，WAL 用例改由 Turso 生成帧并以 32 KiB 全零文件代表未通过自身头部校验的派生索引。测试夹具全部改用 Turso 构造数据库、视图与 WAL。

| 功能 | 测试 |
| --- | --- |
| UTF-8 数据库引用从副本读取且源文件逐字节保持 | `sqlite_resource_references_are_read_from_a_copy_and_source_is_preserved` |
| UTF-16 数据库无法核验时仍拒绝删除 | `utf16_databases_cannot_be_verified_and_still_refuse_deletion` |
| 活动数据库 WAL 不被省略或改写 | `active_database_wal_is_not_silently_omitted_or_modified` |
| 不完整、损坏 WAL 与未核验派生索引 | `incomplete_or_corrupt_wal_and_unverified_index_cannot_allow_deletion` |
| 二进制值与损坏文件 | `unverified_database_values_and_corruption_still_refuse_deletion` |
| DAG 与二进制资源引用同套检查 | `sqlite_resources_protect_dag_and_binary_dependencies_too` |
| 视图定义检查不执行视图 | `sqlite_view_definitions_are_checked_without_executing_the_view` |

仅运行本次改动及直接影响链路的定向检查，未运行全量测试。

相关说明：[Control](../../../agents/control/index.md)、[Turso 存储](../2026-10-19/turso-storage.md)。
