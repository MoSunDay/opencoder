Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# Ontology CLI 与平台能力同步

Ontology 提供平台能力的统一查询和组织入口；CLI 覆盖已有 Ontology API，并增加能力目录、同步状态、差异检查与补同步。

服务端复用原生定义读取逻辑，自动维护实体、目录、明确依赖和固定切面。来源失败保留上次成功快照，生成数据和人工补充分别维护。新增兼容表记录快照、生成对象归属和同步状态；原模块的执行、权限和不可变运行定义保持原有约定。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 全部命令解析、路径及 JSON | `ontology_commands_map_to_native_methods_paths_and_revisions` | `crates/ctl/tests/parse_ontology.rs` |
| CLI 真实 Server 往返 | `cli_catalog_and_data_commands_roundtrip_through_real_server` | `crates/ctl/tests/ontology_server.rs` |
| 类型、属性、Action 与环境增删改查 | `cli_schema_and_environments_support_create_read_update_soft_delete` | `crates/ctl/tests/ontology_server/schema.rs` |
| 实体、关系、切面增删改查与图谱查询 | `cli_entities_relationships_and_aspects_support_crud_and_graph_queries` | `crates/ctl/tests/ontology_server/records.rs` |
| 实体目录创建、挂载、查询、移动、解除与约束 | `cli_directory_tree_mounts_new_and_existing_entities_and_rejects_invalid_moves` | `crates/ctl/tests/ontology_server/directories.rs` |
| 幂等及人工内容保留 | `catalog_is_idempotent_and_preserves_notes_and_manual_relations` | `crates/ontology/tests/catalog.rs` |
| 生成字段保护 | `managed_fields_reject_writes_but_custom_attributes_remain_editable` | `crates/ontology/tests/catalog.rs` |
| 失败回滚及恢复 | `failed_snapshot_rolls_back_changes_and_retains_last_success_on_error` | `crates/ontology/tests/catalog.rs` |
| 自动同步及权限 | `native_changes_automatically_update_catalog_and_keep_read_permissions` | `crates/control/tests/ontology_catalog.rs` |
| 资源代理成功后通知同步 | `forwarded_resource_changes_notify_sync_only_after_success` | `crates/control/tests/ontology_catalog.rs` |
| 来源失败与重启 | `source_failure_keeps_previous_snapshot_and_recovers_after_restart` | `crates/control/tests/ontology_catalog.rs` |
| 并发与退休 | `concurrent_syncs_are_serialized_and_retired_servers_do_not_start_more` | `crates/control/tests/ontology_catalog.rs` |
| 入口登记检查 | `every_public_cli_domain_is_registered_as_a_platform_capability`、`public_control_routes_have_platform_catalog_ownership` | CLI 与控制面定向测试 |
| 六类定义及明确引用 | `six_native_kinds_keep_explicit_references_without_copying_private_configuration` | `crates/control/src/ontology_catalog/source.rs` |
| 检查保持只读 | `reading_a_missing_registered_workflow_does_not_create_source_directories` | `crates/control/src/api/brain_runs/catalog/sources.rs` |
| 检测并修复生成数据偏离 | `check_detects_changed_generated_objects_and_sync_repairs_them` | `crates/ontology/tests/catalog.rs` |
| 页面来源及字段编辑权限 | `shows native capability entrypoints, protects generated values, and keeps notes editable` | `crates/web/spa/src/ontology/pages/entityTypes/EntityDetailDrawer.test.tsx` |
| 实际页面与四种屏宽 | 能力目录、只读字段、说明保存后同步、内嵌 SPA 校验 | `scripts/acceptance/ontology/catalog.py` |

定向验证：

- CLI 命令映射：3 项通过；真实 Server 往返、增删改查、切面与实体目录挂载：4 项通过。目录测试覆盖新实体创建时归属、已有实体挂载和解除后重新挂载、父子目录移动、重复归属与循环拒绝、非空目录删除拒绝及空目录软删除。CI 同时执行命令映射与这些实际操作测试。
- Ontology 能力同步、原有存储与图谱：12 项通过。
- 控制面同步及入口登记：5 项通过；目录相关单元测试 16 项通过，Brain 能力选择及可用性回归 3 项通过。
- Core Ontology 相关测试：3 项通过。
- 前端详情与编辑状态：3 个测试文件、13 项通过；Ontology 类型检查和 SPA 构建通过。
- Core、Control、Ontology、CLI 四个 crate 的 `clippy --all-targets -- -D warnings` 通过；Server 与 CLI 构建通过。
- 隔离 Server 浏览器验收通过：1920、1280、768、390 四种屏宽，各检查四个详情标签，无页面或详情横向溢出；确认生成字段只读，补充说明保存后再次同步仍保留，内嵌 SPA 与当前构建逐文件一致。回执与 16 张截图位于 `/tmp/ontology-catalog-ui-20261020-4`，可通过 `scripts/acceptance/ontology/catalog.py --server <binary> --root <new-evidence-directory>` 复验。

本次只执行上述定向检查，不表示全量测试通过；不发布生产，不修改已有鉴权数据。

相关：[功能说明](../../ontology/index.md) · [CLI](../../../rules/10-ontology-cli.md) · [规则](../../../rules/09-ontology-capabilities.md)
