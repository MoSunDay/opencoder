Commit: 17ca79c3b55e06c4a2b677a16302a9ec1ed81b19

# 历史会话排序索引维护

新增只读预览及有界事务维护入口，为默认会话清单创建按活动时间、ID 排序的部分索引，并刷新会话查询统计。数据库文件身份、表结构、既有索引定义、WAL 模式及执行后的查询计划均须通过；失败回滚。应用记录、会话消息、鉴权数据和 schema 版本不被修改。此工具不改变节点执行状态，也不替代结果验收。

| 功能 | 测试 |
| --- | --- |
| 全部分页、相同时间顺序、会话和消息保留、幂等执行 | `test_preview_then_apply_preserves_all_pages_rows_and_is_idempotent` |
| 文件身份变化及索引定义冲突拒绝 | `test_changed_identity_and_conflicting_index_are_rejected` |
| 执行后验证失败回滚 | `test_verification_failure_rolls_back_index` |
| 写锁争用及无效预览拒绝 | `test_busy_writer_and_invalid_preview_leave_schema_unchanged` |
| 应用行写入和鉴权表读取拒绝 | `test_authorizer_denies_application_writes_and_credential_reads` |
| CLI 预览、执行及回执 | `test_cli_preview_and_apply_write_receipts` |
| 回执不能覆盖数据库 | `test_cli_refuses_receipt_over_database` |
| 引擎标记数据库在打开前拒绝 | `test_engine_marker_rejects_preview_and_apply_before_sqlite_open` |

此前本地和目标环境的 7 项测试通过；本次增加引擎标记拒绝后，本地按 `PYTHONPATH=scripts/maintenance python3 -m unittest session_index.test_index -v` 复核 8 项通过。测试使用临时 WAL 数据库；未修改 Rust，未运行全量回归。实际数据库执行和连续运行验收另存私有回执，不从测试通过推断线上稳定。
