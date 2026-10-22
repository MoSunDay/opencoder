Commit: c53066e0c2181a21709c6edeb5de86b5a0ee61b9

# 共享结构迁移与报告状态兼容

## 问题与改动

- 候选启动给共享 Ontology 库建表，可能使旧 Turso 事务后续持续返回 schema changed。普通启动改为结构检查；维护在停止旧服务并验证备份后授权指定候选，事务内完成建表、系统数据及后置校验。
- SQL 定义由 Rust 初始化和 Python 发布检查共用。存在同名表不表示迁移完成；列、主键、CHECK、索引、正文目录绑定和默认数据空间均参与核验。冲突对象不覆盖，缺失对象仅在维护时补齐。
- 独立 ACK writer 使用数据格式 12；格式 11 保留读取过渡能力。新控制器验证 reader/writer 和引擎；旧控制器通过已有格式限制拒绝不兼容交接。
- `ontology_catalog_schema` 进入二进制构建信息和清单，缺省 0；候选需要结构 1 时在暂存及启动前检查。信号回滚先检查兼容性再安装控制器；直接回滚先检查再修改上传设置。

## 验收对应

| 行为 | 定向用例或入口 |
| --- | --- |
| 缺表拒绝、维护补齐、重试 | `ordinary_open_rejects_old_schema_without_creating_tables` |
| 正常启动不执行 DDL | `valid_schema_checks_do_not_write_or_require_a_writer` |
| 同名对象结构冲突 | `conflicting_table_is_not_replaced_or_mistaken_for_complete` |
| 缺索引、正文根不可重绑 | `missing_index_requires_maintenance_and_binding_is_never_rewritten` |
| 事务内失败无部分建表 | `failed_seed_rolls_back_all_new_tables` |
| 版本、路径、端口、备份与发布互斥 | `startup_requires_exact_unopened_maintenance_identity_and_backup` |
| 预热不接受陈旧维护凭据 | `candidate_prewarm_only_validates_schema_even_with_stale_migration_receipt` |
| 格式和清单拒绝、只读预检 | `test_report_state.py`、`ontology/test_schema.py` |
| 信号拒绝发生在控制器安装前 | `test_incompatible_signal_rollback_does_not_install_controller` |
| 真实旧连接、维护恢复、ACK 跨版本 | [隔离验收](../../../scripts/acceptance/release_compatibility/README.md) |

实际候选的源码、程序摘要、测试结果及内存采样保存在验收输出的 JSON 和日志中；局部测试不代表全量通过。生产发布与线上观察单独执行。

最终 Linux 候选通过真实旧、新版本的 Schema/ACK 验收，包含迁移事务中进程退出、提交后缺失回执的重试，以及读版本再次重启后的确认记录保留。七个发布程序构建信息一致，发布包及压缩归档均已校验。隔离 runc 交接完成 17 个场景，232 次连续提交无失败；最终发布脚本定向回归 36 项通过。

## 兼容边界

旧库缺少目录结构时必须使用维护流程；正常重启不会隐式迁移。开放写入前保留同一备份及恢复能力，开放后拒绝恢复旧备份。历史 Runtime 的归属、执行数据和鉴权数据不随本次修复改写。

格式检查阻止旧控制器授权危险版本交接，但管理员直接运行历史脚本仍可能更新其控制器 unit；正式发布使用随候选验证的控制器快照。

相关说明：[发布流程](../../../docs/smooth-release.md)、[Ontology](../../ontology/index.md)、[Agent 平台](../../agent-platform/index.md)。
