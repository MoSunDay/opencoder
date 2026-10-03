Commit: 347a6bdfee28570f5c5cf9e2e1891d563cdf1bb7

# Ontology 与可滑动分类导航

## 范围与行为

- 项目、Agent、Ontology、节点使用完整宽度的分类标签，空间不足时横向滚动；桌面与移动端均支持当前标签自动显示和键盘选择。
- 通用 Ontology 页面使用 OpenCoder 身份与 antd 组件：实体、目录、类型、属性、Action 配置、关系、环境、图谱和命名切面。Action 仅保存配置。
- Server 新增独立 SQLite 域，User/Root 只读，Admin 管理；写请求统一事务、幂等与 revision 校验，断开请求不会取消已接纳写入，退休等待排空。
- 正文保留不可变版本并通过本服务第四个只读 NFS 导出；独立资源进程持续持有端口。数据库与正文一起备份和恢复，支持外部正文根。
- 数据格式提升至 3，首次升级使用维护流程；正文导出默认关闭，需显式启用。本项完成代码与隔离验收，未部署生产。

## 测试覆盖

| 功能 | 测试名或真实验收入口 | 文件 |
| --- | --- | --- |
| 完整标签、滚动、键盘与宽度变化 | `keeps complete labels, scrolls overflowing tabs, and supports keyboard selection`、`keeps the active category visible when the tab bar shrinks` | [categoryTabs.dom.test.jsx](../../../crates/web/spa/src/shell/categoryTabs.dom.test.jsx) |
| 图谱筛选、多个中心与跨类型邻居 | `directional_depths_are_independent_and_multicenter_ranges_are_unioned`、`expanded_observation_uses_selected_types_as_seeds_and_crosses_type_boundaries` | [graph/tests.rs](../../../crates/ontology/src/api/graph/tests.rs) |
| 类型、关系、目录环与切面 | `graph_aspects_relationship_scopes_and_directory_cycles_use_live_metadata` | [graph.rs](../../../crates/ontology/tests/graph.rs) |
| 原子创建、幂等、失败回滚与重启历史 | `creation_is_atomic_idempotent_and_preserves_text_history_after_restart` | [storage.rs](../../../crates/ontology/tests/storage.rs) |
| 环境隔离、必填属性与 revision | `environments_and_required_attributes_are_isolated_and_revision_checked` | [storage.rs](../../../crates/ontology/tests/storage.rs) |
| 普通用户只读全部域 | `ordinary_roles_read_all_domains_but_cannot_modify_them` | [graph.rs](../../../crates/ontology/tests/graph.rs) |
| 2048 维余弦搜索与环境分区 | `vectors_search_real_cosine_values_and_keep_environment_partitions` | [graph.rs](../../../crates/ontology/tests/graph.rs) |
| 正文不可覆盖与并发写 | `immutable_write_is_idempotent_and_rejects_overwrite`、`concurrent_writers_never_replace_the_winner` | [text_store.rs](../../../crates/ontology/src/text_store.rs) |
| 客户端断开与退休排空 | `disconnected_clients_do_not_cancel_accepted_writes_and_shutdown_waits` | [mutations.rs](../../../crates/ontology/src/mutations.rs) |
| 平台认证、独立库与第四个导出 | `authenticated_ontology_routes_and_resource_export_share_files_without_exporting_database` | [control/tests/ontology.rs](../../../crates/control/tests/ontology.rs) |
| 重名表单字段与失败重试 | `labels target their own field when other retained modals use the same field name`、`keeps the entered value after an unsuccessful save so the user can retry` | [ModalForm.test.tsx](../../../crates/web/spa/src/ontology/ui/__tests__/ModalForm.test.tsx) |
| 外部正文根备份恢复与端口约束 | `test_snapshot_includes_external_text_root_and_stopped_recovery_restores_both`、`test_four_export_ports_are_distinct_and_ontology_rejects_writable_options` | [test_ontology.py](../../../scripts/platform/rolling_tests/test_ontology.py) |
| 五个页面完整浏览器操作及实际只读 NFS | 类型、属性、Action、实体正文、关系、切面、环境隔离，NFS 以 rw 请求挂载后写入仍被服务拒绝 | [ontology/main.py](../../../scripts/acceptance/ontology/main.py)、[browser.mjs](../../../scripts/acceptance/ontology/browser.mjs) |
| 全站范围和失败记录续跑 | `every registered page has a functional case, separate from responsive snapshots`、`previous failed checks stay failed until rerun and their evidence is retained` | [scope.test.js](../../../scripts/acceptance/ui/scope.test.js)、[resume.test.js](../../../scripts/acceptance/ui/resume.test.js) |
| 切换、回滚后正文、历史、切面与 NFS 连续性 | `ontology-text-and-aspect-survive-switch-and-rollback` | [domain_checks/ontology.py](../../../scripts/acceptance/smooth_release/domain_checks/ontology.py) |
| 15 分钟完整观察与失败回执 | `test_observation_requires_the_entire_duration_and_runs_real_submissions`、`test_domain_failure_keeps_the_receipt_unsuccessful` | [test_observation.py](../../../scripts/acceptance/smooth_release/tests/test_observation.py) |

## 验证结果

- `cargo test --workspace`：446 组原始 `test result` 通过数求和为 **5639 passed / 0 failed**；没有删减失败断言或跳过失败用例。完整输出位于 `/root/.cache/opencoder-e2e/20261002-ontology-verification/workspace-tests.log`。
- `cargo clippy --workspace --all-targets -- -D warnings`、workspace 构建和 `cargo fmt --all -- --check` 通过。
- SPA：130 个测试文件，**960 passed / 0 failed**；Ontology TypeScript 检查、构建、最终注释修正后的产物漂移检查通过。
- 备份恢复专用 Python 回归 2 项、平滑切换验收与观察辅助回归 12 项、全站覆盖及续跑辅助 Node 回归 6 项通过。
- 全站验收 15 项检查通过，覆盖全部 **15 个页面**、1920/1280/768/390 四种宽度、主要弹窗、真实功能和 Server TUI。回执：`/root/.cache/opencoder-e2e/20261002-ontology-global-ui-v2/receipt.json`；Ontology 浏览器与实际挂载回执位于其 `ontology-3/result.json`，清理成功。
- 平滑切换及回滚：18 个场景、79 次连续提交，无请求失败；保留实际 OCI 容器及 Shell 进程，Ontology 正文、历史与切面连续可读。随后完成 **900.000086 秒**观察并完成 57 个真实任务样本。回执：`/root/.cache/opencoder-e2e/20261002-ontology-release-observation/opencoder-smooth-r1vzxgg6/result.json`，专属资源清理成功。

构建信息为 `fd9fdc34a732e0fab72d1a664792c7654528ab63` 的 dirty 开发构建，六个二进制、镜像 runner 与 SPA 配套校验通过。SPA SHA256：`083c3f2b136855f6446c3a2be37463d301803ff8c8c8ff1dcbf7bcc4b1ffc440`。测试使用临时数据库、合成凭据、私有挂载空间；持续进程演练使用隔离 tmpfs，仅作为功能和竞态验证，不作为生产磁盘延迟或持久性结论。共享仓库其他并行改动不纳入本项冻结验收范围。

原始 Rust/SPA 构建与测试日志及 SHA256 清单保存在 `/root/.cache/opencoder-e2e/20261002-ontology-verification/`；不将临时测试数据库或凭据提交到 Git。

## 相关

- [Ontology 能力](../../ontology/index.md)、[Ontology 逻辑](../../../agents/ontology/index.md)
- [Web](../../../agents/web/index.md)、[控制面](../../../agents/control/index.md)、[Server](../../../agents/server/index.md)
- [维护与备份](../../../docs/smooth-release.md)、[全站 UI 验收](../../../rules/05-ui-acceptance.md)
