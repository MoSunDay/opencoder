# Ontology 按实体读取属性和正文版本

批量创建或读取实体时，原查询反复加载同一环境的全部结构化属性和正文版本。现在显式选择已有实体索引限定读取范围（缺少统计信息时也不会因排序选择全环境扫描）；普通实体加入目录只查自身归属，目录实体继续执行环校验。未改变 HTTP 契约、存储格式和 revision 校验。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 多实体属性隔离、正文历史与目录归属 | `entity_reads_preserve_scope_and_revisions_with_unrelated_values` | `crates/ontology/tests/entity_lookup.rs` |
| 属性和文本查询固定实体索引 | `entity_value_queries_use_the_entity_indexes_without_statistics` | `crates/ontology/src/database/store.rs` |
| 上下游查询使用两个现有索引 | `anchored_relationship_query_uses_both_endpoint_indexes` | `crates/ontology/src/database/model/relationships.rs` |
| 创建幂等、事务与正文历史 | `creation_is_atomic_idempotent_and_preserves_text_history_after_restart` | `crates/ontology/tests/storage.rs` |

- `cargo test --locked -p opencoder-ontology`：40 passed。
- `cargo clippy --locked -p opencoder-ontology --all-targets -- -D warnings`：零警告。
- 测试范围限于 Ontology 及其接口；未运行全工作区测试。
