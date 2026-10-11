Commit: 6988229a240f48ce3fce0f2eb17d2d7d4c32b7bf

# 注册内容清理与删除保护

Server 增加 `DELETE /api/brain/plan-defs/:id`，删除计划的全部保存版本及能力元数据。其他保存计划（含旧版本）或未结束 Brain 运行仍引用该计划时返回 409；已结束运行的冻结快照继续保留。

Server 删除能力时同时清理目标绑定和能力元数据，重试可收尾已部分删除的数据。保存计划、启动 Brain 和删除操作通过跨进程目录锁协调，防止校验后新增引用。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 删除全部计划版本及元数据，重复删除 | `deleting_brain_plan_removes_versions_and_catalog_metadata` | `crates/store/tests/brain_versions.rs` |
| 精确匹配计划 ID，保留相似名称 | `deleting_plan_uses_exact_case_sensitive_prefix_and_all_versions` | `crates/store/tests/brain_versions.rs` |
| API 删除后清除列表与能力库入口 | `deleting_saved_plan_removes_versions_and_catalog_entry` | `crates/control/tests/e2e/layered_api/plans.rs` |
| 正在运行的保存计划不能删除 | `deleting_saved_plan_is_blocked_while_a_run_uses_it` | `crates/control/tests/e2e/layered_api/plans.rs` |
| 保存父计划、未派发的嵌套计划引用保护 | `deleting_plan_rejects_saved_parent_and_unlaunched_nested_run` | `crates/control/tests/e2e/layered_api/plans.rs` |
| 保留旧格式计划中的能力引用 | `deleting_capability_preserves_legacy_saved_plan_references` | `crates/control/tests/e2e/brain_api/capabilities.rs` |
| 能力删除保护覆盖旧版本分页和活动运行 | `deleting_capability_preserves_saved_and_active_consumers` | `crates/control/tests/e2e/brain_api/capabilities.rs` |
| 能力删除清理绑定、元数据及重试 | `capability_crud_search_and_target_binding`、`deleting_capability_clears_orphan_metadata_on_retry` | `crates/control/tests/e2e/brain_api/capabilities.rs` |

## 验证

- 全量回归：`cargo test --workspace --no-fail-fast -- --nocapture` → 5757 passed / 0 failed / 8 ignored（仓库已有的手工或可选用例；本次未新增忽略）。完整日志保存在维护备份目录。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo build --workspace --all-targets`：通过。
- `cargo fmt --all -- --check`、`scripts/check-spa-drift.sh`：通过。
- 真实容器测试使用独立的临时文件系统和受限 CPU 数量；测试盘允许容器设备文件，断言保持不变。

相关索引：[Control](../../../agents/control/index.md)、[Brain 工作台](../../brain/index.md)。
