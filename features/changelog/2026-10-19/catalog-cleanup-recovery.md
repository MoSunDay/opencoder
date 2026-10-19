Commit: 10fce90760adbfbea493ac0203a208e21adb5a2b

# 目录引用检查与删除结果恢复

目录删除检查全部 Agent 工具与技能文件，不再按扩展名排除合法文件。不能读取或确认内容、资源根失效、软链接、特殊文件及超限文件均拒绝删除。

健康接口声明 `catalog_guard: 2`。先发布 Server，再使用新维护工具；新工具连接旧版保护时，在扫描和删除之前拒绝执行。

删除超时和断连保留待确认回执；重跑先查询原对象，查询不可用时继续保留原记录，不重复删除。旧版误记为跳过的完整删除意图也会补查。备份在完整写入、同步与校验后才正式保存；恢复必须匹配回执摘要，不能覆盖同名不同内容的对象。历史、鉴权数据、工作区及资源分发协议不变。

| 行为 | 定向测试 |
| --- | --- |
| 任意合法工具扩展名均阻止所引用对象删除 | `every_legal_tool_extension_protects_referenced_pools_and_definitions`、`test_every_extension_protects_pool_in_the_real_inventory` |
| 未知编码、内容、根目录和读取失败拒绝清理 | `unreadable_or_unverified_agent_data_never_allows_catalog_deletion`、`test_unknown_data_roots_links_and_io_failures_are_rejected` |
| 超时、断连后补查并恢复原对象 | `test_transport_errors_remain_pending_then_reconcile_and_restore` |
| 5xx 回包及不可用查询不会误判或重复删除 | `test_5xx_acknowledgement_is_resolved_by_readback`、`test_unavailable_readback_does_not_repeat_delete` |
| 旧回执与新扫描失败不阻止已有结果确认 | `test_legacy_skipped_intents_and_inventory_failure_preserve_recovery` |
| 备份中断重试、摘要不符拒绝恢复 | `test_backup_interruption_does_not_publish_a_partial_anchor`、`test_changed_backup_never_becomes_a_restore_anchor` |
| 待确认回执产生非零退出码 | `test_cli_reports_pending_and_returns_failure` |
| 新维护工具在读取或删除数据前拒绝旧版 Server | `test_old_server_is_refused_before_reading_or_deleting_data` |

定向验证：`catalog_guard` 共 15 项通过；资源摘要阻塞线程用例 1 项通过；维护脚本 19 项在本地与部署环境 Python 3.7.3 均通过。`cargo clippy --locked -p opencoder-control --all-targets -- -D warnings` 零警告，相关 Rust 格式及文档链接检查通过。未运行全量测试。

发布验证：7 个程序的编译提交与摘要一致；信号发布、回滚、再次发布及 900 秒持续执行观察通过，410 个验收任务全部完成。接收耗时 P95 为 1.38 秒，最大接收耗时 9.93 秒，最大调度间隔 10.09 秒；539 次就绪检查无失败，事件流恢复均在 1 秒内。

最后一个验收任务结束后，再完成 900 秒只读稳定观察：177 次检查均通过，版本、配置摘要、数据库结构和关键进程保持一致，运行程序摘要与发布包一致。目录复核未发现重复注册绑定、完全相同的 DAG 定义或二进制池，原有历史快照保持不变。

相关说明：[Control](../../../agents/control/index.md)、[DAG](../../dag/index.md)、[维护入口](../../../scripts/maintenance/catalog_cleanup/README.md)。
