Commit: 51cb1e361e0774effa9f4eb38e93cc75e432b068

# 里程碑调度与项目指派发布检查收口

## 背景

分层计划的旧节点仍校验层间路径；项目 TODO 看板、监控和数据库升级也需要满足实际运行与重复部署要求。

## 变更

- 新建分层计划自动补齐内部前进与回退路径；旧计划保留原路径，模型仍依据当前执行证据自主选层。
- 指标抓取使用独立凭据，仅可访问 `GET /metrics`；滚动发布在启动前核验凭据文件，监控注册先验通路并保留可定向恢复的原配置。
- MySQL/StarRocks 存量 TODO 排序升级用 `-1` 标记待回填位置，重复升级保留用户设置的第 0 位；Web API 拒绝负数排序位置。
- Operator 执行写出最后一条助手正文，供 TODO 指派结论同步；TODO 抽屉在窄屏收缩到视口内。
- 设备用例提示词的 v2 历史版本恢复原文，新版本仍由 v3 提供。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 内部路径及旧计划保留 | `rollback_paths_cover_every_executed_layer_without_changing_old_versions` | `crates/core/src/brain/layered/plan.rs` |
| 计划保存和重试 | `new_plan_save_is_idempotent_and_keeps_paths_for_retained_runtimes` | `crates/control/tests/e2e/layered_api/plans.rs` |
| 监控配置凭据 | `test_distinct_credential_is_required_before_release` | `scripts/platform/rolling_tests/test_metrics_config.py` |
| 监控注册与恢复 | `test_only_metrics_get_is_allowed`、`test_journal_keeps_first_anchor_across_retry`、`test_restore_changes_only_registered_deployment_fields` | `deploy/monitoring/test_register.py` |
| SQL 升级中断与重试 | `mysql_project_upgrade_contract`、`starrocks_project_upgrade_contract` | `crates/store/tests/sql_project_upgrade.rs` |
| TODO 位置 API | `goal_milestone_todo_crud_contract` | `crates/web/tests/web_project.rs` |
| Operator 结果正文 | `operator_session_runs_prompt_to_idle`、`non_admin_role_gates_the_surface` | `tests/operator_e2e/` |
| Agent、Operator 结论及手机端 | `--workbench-only` 浏览器验收 | `scripts/acceptance/project/main.js` |

- 前端：900/900 测试通过，构建与 SPA 漂移检查通过。
- Python：滚动发布配置 52 项、监控注册 3 项测试通过。
- MySQL/StarRocks 特性编译通过；现场未提供两个数据库的测试 DSN，因此未运行数据库实例集成测试。
- 浏览器验收：项目层级、TODO 指派、Agent 与 Operator 结论回写、390px 抽屉边界和无页面异常均通过。
- 全量 Rust 回归：`cargo test --workspace -j8` → 430 个套件、5650 passed / 0 failed / 8 ignored。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；`cargo build --workspace -j8` 通过。

当前规则见 [大脑工作台](../../brain/index.md)、[项目工作台](../../project/index.md)和[Agent 调度平台](../../agent-platform/index.md)。
