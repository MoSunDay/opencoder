Commit: 79eee7111a7672fdfaf23b3adc575015e9e3a644

# TODO 模板只保留最近 10 个版本

TODO 模板的版本目录此前无限增长，每次保存/派生新版本都会永久留下一个旧版本目录。现在发布新版本时自动执行保留策略：一个模板最多保留最近 10 个版本，当前版本（current）优先级最高、永不被清理，并且也占用这 10 个名额之一——单版本模板计 1/10，不会出现「10 个历史版本 + 1 个当前版本」的额外豁免。

淘汰按版本的 `created_at` 从最旧开始（数组下标作并列裁决），`todo.json` 的 `versions` 列表与磁盘目录同步收敛：元数据先落盘、目录后删除，崩溃最多留下无引用的孤儿目录（`next_version` 已兼容），不会出现元数据指向已删目录。`new-version` 响应新增 `pruned` 字段列出本次淘汰的版本名；SPA 版本列表同步展示保留策略提示。被淘汰的版本不再出现在模板详情、版本选择和运行入口，读取其文件返回 404。手动删除版本与删除模板行为不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 超过 10 个版本时按时间淘汰最旧、元数据与磁盘一致、被淘汰版本 404 | `retention_keeps_recent_ten_versions_and_prunes_oldest` | `crates/web/tests/web_todo_templates.rs` |
| current 回拨到最旧版本本身不触发清理，下次发布按新 current 淘汰 | `retention_pinned_current_is_untouched_until_next_fork` | `crates/web/tests/web_todo_templates.rs` |
| 保留上限与 current 保护（含 current 为最旧、created_at 乱序、缺失字段） | `pruning_keeps_at_most_ten_with_current_protected` / `pruning_orders_by_created_at_and_tolerates_missing_fields` | `crates/web/src/api_todo_util.rs` |
| 版本列表保留策略提示 | `expands a row and dispatches a run for the version` | `crates/web/spa/src/todoPanel.dom.test.jsx` |

- 定向回归：`cargo test -p opencoder-web`，62 个测试目标 299+ 项通过、0 失败；`cargo test -p opencoder-control --test e2e todo_templates`，4 passed。
- `cargo clippy -p opencoder-web --all-targets` 通过；`cargo fmt -p opencoder-web` 已应用。
- `npm run build` 通过，提交追踪的 SPA 产物与当前源码同步。

相关：[TODO 功能](../../todos/index.md)、[todos 模块](../../../agents/todos/index.md)、[Web 模块](../../../agents/web/index.md)。

## Release

- 2026-09-16 发布上线:`rel-79eee7111a7672fdfaf23b3adc575015e9e3a644`(前序 `rel-071c3aca`)。发布分支 `release/20260916-todo-template-retention` 合入部署链 `071c3aca`(act/plan 暂存、会话悬停删除、DAG 更新时间、准入上报同步)与 main(本功能、节点 workdir 调度、say 无损送达、会话工作区拆分),并在干净 worktree 重建 `dist/static/app.js`(eb6ca791 的产物曾被并行会话未提交源码污染,已用 `79eee711` 修正,check-spa-drift 无漂移)。
- 全量验证:`cargo clippy --workspace --all-targets -- -D warnings` 零警告;`cargo test --workspace` 400 个目标 5,250 项通过、0 失败(日志 `/tmp/rel-cargo-test.log`);`cargo build --workspace` 通过;SPA `npm test` 101 文件 712 项通过;python 套件 signal_tests 12 + rolling_tests 19 + scripts/platform 19 + smooth_release 4 全部 OK。
- 验收:`scripts/acceptance/smooth_release/live.py --signal --observe-seconds 900` 结果 PASS(证据:`/var/tmp/release-live-7064ef23f7edfa6b`);真实 TODO 链与 DAG 长任务通过,SSE 恢复 0.158s,持续提交/调度间隔最大 0.40s,观察满 900 秒 171 样本;旧 Runtime 已退役,仅新版本 Runtime 运行。
