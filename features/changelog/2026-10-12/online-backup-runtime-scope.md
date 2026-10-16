Commit: af3b665adede007c9d365a10ef2dff215993e05a

# 在线备份避开 Runtime 只读副本

在线备份递归遍历所有 `.db`，会进入固定资源、OCI 镜像和容器根中的知识挂载副本，重复扫描嵌套历史 Runtime。现仅在受管 Runtime 布局内，在读取子目录前排除不可变输入；真实数据库与 Ontology 备份保留。停服全量文件复制及嵌套数据库备份不变。

已知 `.cleared-executions-*` 归档可应用受管布局判定，用户目录不能凭同名、深度或复制的回执被排除。备份清单列出 `excluded_database_trees`，中断仍仅保留未完成 staging。恢复故障测试的 HTTP 桩提供真实 Host 代次响应，覆盖新就绪检查涉及的外部故障断点。

## 功能与测试

| 功能 | 测试名 | 文件 |
|---|---|---|
| 读取前排除副本，真实嵌套 SQLite 保留 | `test_online_prunes_replicas_before_any_directory_read_and_keeps_real_databases` | [test_snapshot.py](../../../scripts/platform/rolling_tests/backup/test_snapshot.py) |
| 同名用户数据与完整同形回执不误排除 | `test_online_preserves_same_names_in_user_data_and_unrecognized_container_trees` | [test_snapshot.py](../../../scripts/platform/rolling_tests/backup/test_snapshot.py) |
| 可写或无法确认的 OCI 保留，损坏回执不猜测 | `test_writable_or_unrecognized_oci_root_is_never_pruned`、`test_invalid_oci_receipt_fails_without_publishing_or_guessing_exclusions` | [test_snapshot.py](../../../scripts/platform/rolling_tests/backup/test_snapshot.py) |
| 停服完整文件、所有嵌套数据库及中断私有 staging | `test_stopped_backup_preserves_all_nested_databases_and_execution_files`、`test_online_failure_keeps_only_private_incomplete_staging` | [test_snapshot.py](../../../scripts/platform/rolling_tests/backup/test_snapshot.py) |
| 外部故障断点续跑及嵌套数据库恢复 | `test_every_external_switch_boundary_can_resume_without_duplicate_release`、`test_stopped_backup_preserves_nested_databases_and_retry_never_reuses_partial` | [test_recovery.py](../../../scripts/platform/rolling_tests/test_recovery.py) |
| 外部 Ontology 正文快照与恢复 | `test_snapshot_includes_external_text_root_and_stopped_recovery_restores_both` | [test_ontology.py](../../../scripts/platform/rolling_tests/test_ontology.py) |

## 验证

- 原无边界扫描：新增 5 项测试中 1 项失败，明确进入不可变副本；未限制归档父名的版本在完整同形用户目录测试中失败。
- `python3 -m unittest discover -s scripts/platform/rolling_tests/backup -p test_snapshot.py -v`：6 passed。
- `PYTHONPATH=scripts/platform/rolling_tests python3 -m unittest scripts.platform.rolling_tests.test_recovery scripts.platform.rolling_tests.test_ontology scripts.platform.rolling_tests.test_deployment scripts.platform.rolling_tests.test_probes -v`：47 passed。
- 只运行备份、Ontology、发布门槛与故障续跑的定向测试；未修改 Rust，Clippy 不适用。生产备份和发布验收另行执行。

相关：[Server](../../../agents/server/index.md)、[调度平台](../../agent-platform/index.md)、[备份说明](../../../docs/smooth-release.md)。
