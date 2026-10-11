Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# 项目和专项当前概述

项目和专项在现有人员设置之外增加当前概述与概述更新时间，支持列表筛选、详情阅读和表单编辑。概述按关键进展、核心工作、剩余卡点整理，时间只在明确提交正文时更新。

`project initiatives summary-context` 读取全部 TODO、全部历史关联执行及原生过程，继续读取分页和省略正文，重复执行只读取一次。`project goals summary-context` 汇总所属全部专项，提示缺失概述和概述之后的 TODO 变更。Codex / Operator 整理后通过现有 PATCH 写入 `current_summary`；读取错误保留来源，概述不代替验收，不修改看板，不增加后台收集。

现有项目、专项表各增加正文和时间字段，schema 升至 37，发布数据契约为 8。旧格式须维护升级；升级和回退脚本识别新 schema，沿用对认证数据的完整性核验。新增一张共享概述历史表，不新增运行配置变量。每个对象保留最近 10 个版本；保存、清空、回滚都留记录，回滚新增版本并记录来源。正文与历史写入、清理保持事务一致，Web 与 ctl 提供查看和回滚入口。

相关说明：[用法](../../../docs/project-summaries.md)、[功能](../../project/index.md)、[项目规则](../../../rules/07-project-module-contract.md)、[ctl](../../../agents/ctl/index.md)、[Store](../../../agents/store/index.md)、[Web](../../../agents/web/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 保存、清空、独立时间及事务回滚 | `summaries_have_independent_timestamps_and_clear_without_changing_progress` | `crates/store/tests/project_summary.rs` |
| 旧库升级、记录保留及重开 | `schema_36_upgrade_preserves_records_and_reopen_preserves_summaries` | `crates/store/tests/project_summary.rs` |
| 两类对象 HTTP 读写与权限 | `project_and_initiative_summary_http_roundtrip_and_permissions` | `crates/control/tests/e2e/project_api/summary.rs` |
| 全部历史执行、Brain 分页、长正文、离线提示 | `initiative_reads_all_assignments_pages_full_messages_and_reports_offline_sources` | `crates/ctl/tests/project_summary.rs` |
| 项目汇总、CLI 分发、错误退出码、异常游标 | `project_collects_every_initiative_and_warns_about_missing_or_outdated_summaries`、`summary_context_cli_parses_dispatches_and_keeps_http_exit_codes`、`non_advancing_process_cursor_is_reported_without_hanging` | `crates/ctl/tests/project_summary.rs` |
| 展示、编辑、清空、筛选及刷新时保留新概述 | `summary.dom.test.jsx` 的 4 个用例 | `crates/web/spa/src/project/summary/summary.dom.test.jsx` |
| 新数据格式隔离旧版本 | `project_summary_schema_requires_maintenance_from_previous_formats` | `crates/core/src/fleet/release.rs` |
| 维护回退保留认证数据 | `test_summary_schema_inventory_and_rollback_preserve_credentials` | `scripts/platform/maintenance_tests/test_project_summary.py` |
| 10 版本保留、回滚、重开、删除与事务失败 | `summary_history_retains_ten_versions_and_restore_appends_without_touching_other_fields`、`history_write_failure_rolls_back_summary_and_restore` | `crates/store/tests/project_summary_history.rs` |
| 历史接口、读写权限与已清理版本 | `summary_history_is_bounded_readable_and_restorable_with_write_permission` | `crates/control/tests/e2e/project_api/summary.rs` |
| ctl 历史与回滚命令 | `summary_version_commands_parse_and_target_the_matching_scope` | `crates/ctl/tests/project_summary.rs` |
| 历史 UI、确认回滚、失败保留、权限与过期响应 | `history.dom.test.jsx` 的 4 个用例 | `crates/web/spa/src/project/summary/history.dom.test.jsx` |

## 验证

- `cargo test -p opencoder-store --test project_summary --test project_summary_history -- --test-threads=4`：4 passed / 0 failed。
- `cargo test -p opencoder-control --test e2e project_api:: -- --test-threads=4`：21 passed / 0 failed。
- `cargo test -p opencoder-cli --test project_summary -- --test-threads=4`：5 passed / 0 failed。
- SPA 定向回归：概述展示、版本历史、归属编辑、项目页面共 4 个文件、16 个用例通过。项目浏览器验收覆盖 1920、1280、768、390 宽度、修改记录和确认恢复，无页面异常或历史面板溢出。
- 发布相关：数据格式契约用例通过；新增历史表的维护回退用例通过，保留认证数据；滚动发布脚本 67 个用例通过。
- `cargo fmt` 检查及 Store、Web、Control、CLI、Project 五个 crate 的 `cargo clippy --all-targets -- -D warnings` 通过，零警告。
- `cargo build -p opencoder-server -p opencoder-cli` 与 SPA 构建通过；两个二进制的 SPA 摘要与验收产物一致，四个版本命令帮助入口可用。
- 按最终改动范围收尾；较早完成的 SPA 全量与全站布局检查未在版本支持追加后重复运行，未完成的 Rust 全量回归已按更新后的仓库规则停止。
