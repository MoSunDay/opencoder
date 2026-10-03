Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# 正文存储保护与固定源码交付

Ontology 数据库固定首次初始化的正文根。不同目录使启动失败，空库也不能重绑，规范化后的同一路径可复用。维护预检在任何服务操作前检查目录绑定，避免升级后正文历史变成不可读。

配置基础模块保留在 `config/`，运行配置放入 `config/runtime/`；两组各九个逻辑文件，公共类型和 JSON 字段保持原值。SPA 构建严格使用锁文件安装依赖，构建产物须通过重新构建比较。Ontology 验收支持新的仓库外证据目录，并拒绝已有目录、仓库目录和生产状态目录。

交付以固定源码和成套构建产物为准，共享工作区之后的改动不沿用已有验收结果。

## 测试覆盖

| 功能 | 测试或校验入口 | 文件 |
| --- | --- | --- |
| 拒绝正文根变化，当前与历史正文仍可读 | `changing_text_root_rejects_startup_and_preserves_current_and_historical_content` | `crates/ontology/tests/storage_root.rs` |
| 空库固定目录及同一路径别名 | `an_empty_database_cannot_rebind_while_another_server_still_owns_its_root`、`a_canonical_alias_of_the_same_text_root_can_reopen_the_database` | `crates/ontology/tests/storage_root.rs` |
| 服务操作前拒绝配置重定向 | `test_changed_ontology_binding_rejects_before_any_service_operation`、`test_unchanged_default_ontology_binding_is_accepted_without_requiring_a_new_config_field` | `scripts/platform/maintenance_tests/test_preflight.py` |
| 安全证据目录及实际浏览器、只读 NFS | `EvidenceRootTests`、Ontology UI 验收 | `scripts/acceptance/ontology/test_main.py`、`scripts/acceptance/ontology/main.py` |
| 锁文件与产物一致性 | SPA 全量测试、严格重建比较、全站 UI | `scripts/build-spa.sh`、`scripts/check-spa-drift.sh`、`scripts/acceptance/ui/main.js` |

- Rust 全量回归：5674 项通过、零失败；七项真实环境用例另行全部通过。
- 格式检查、全工作区全目标 Clippy 零警告、二进制与运行器构建通过。
- SPA 966 项通过，类型检查与严格产物比较通过；四种屏宽和全站十五项 UI 验收通过。
- 发布控制器 127 项通过；实际切换、回滚、进程持续运行及崩溃恢复通过，完成 900 秒隔离观察。受理与调度最大值沿用 30 秒门槛。

[Core](../../../agents/core/index.md) · [Ontology](../../ontology/index.md) · [发布与维护](../../../docs/smooth-release.md)
