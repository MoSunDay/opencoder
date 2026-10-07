# 维护升级只备份会改写的共享数据

维护升级及失败恢复只改写 Server、资源服务、Host 和服务配置；历史 Runtime 与旧节点执行目录始终原地保留。维护备份不再重复复制这些目录内的镜像、会话和产物。普通在线备份仍包含执行数据库。冻结接入、排空任务、停止写入、备份校验、认证数据保护和开放写入后的恢复限制保持原约定。

容量预检和实际备份使用同一份根目录选择，收据列出备份与保留路径。共享数据备份要求写入方已停止。测试夹具修正 systemd 多属性输出，实际验证旧服务在回滚后重新启动。

## 测试覆盖

| 行为 | 测试 |
|---|---|
| schema 33 可只读预检，未知版本拒绝 | `test_project_reference_schema_can_be_inspected_without_changing_the_database` |
| 共享备份拒绝未停服调用 | `test_shared_backup_requires_stopped_writers` |
| 在线备份仍保留执行数据库 | `test_online_backup_still_includes_existing_execution_databases` |
| 容量及维护备份不遍历历史执行树 | `test_capacity_and_shared_snapshot_never_traverse_retained_execution_trees` |
| 迁移失败与重复回滚保留会话内容和 inode，恢复旧服务 | `test_failed_upgrade_and_repeated_rollback_preserve_execution_context_and_inodes` |
| 现有镜像不重复复制，仍检查磁盘余量 | `test_existing_images_are_not_copied_again_and_available_space_is_rechecked` |

维护测试 58 项通过；同轮 Rust 全量回归 5,690 项通过、0 失败、7 项既有手工测试忽略；Clippy 和 workspace 构建通过。本次变更只涉及 Python 发布控制器及文档。证据：`/data00/workspace/artifacts/operator-project-upgrade-20261007/`。

真实维护演练发现 Nginx reload 返回后旧 worker 仍可能短暂接收请求。维护入口的关闭、回滚恢复与重开现在确认旧 worker 已关闭监听 socket，保留已有节点长连接；隔离演练读取自己的 Nginx PID。新增测试覆盖切换前禁止停服、恢复前禁止报告完成，以及长连接不阻塞切换。维护主套件 58 项、拓扑套件 15 项、滚动发布 66 项、维护演练工具 18 项通过。
