Commit: 3e870567ca1d67d0d27ecab4c8550f64a9c2aca9

# Operator 默认规则与工作区维护发布

## 发布范围

按用户授权发布冻结时的全部工作区改动，包括 Operator 默认规则、项目人员关系与当前概述，以及本次定向验证发现的修复。版本为 `rel-3e870567ca1d67d0d27ecab4c8550f64a9c2aca9`，协议 10、数据格式 8。冻结后其他并行开发的改动保留在主工作区，不属于此发布包。

Operator 每轮携带单一任务的范围、交付与结束规则，以及当前会话实际使用的 skill 目录；原生会话和 Codex 的续聊、恢复、派生均覆盖。功能与测试入口见 [默认提示词](operator-default-prompts.md)、[审查修复](operator-review-remediation.md)。

## 维护与数据

- 使用维护窗口关闭接入，等待已接纳工作自然结束；停止写入服务、保存一致性备份后升级，完成验证再恢复接入。
- schema 从 v35 升级到 v37，原项目行及已有字段保持一致；Server 与资源库的鉴权数据及数据库 inode 保持不变。原有 4 个节点全部恢复在线就绪，3 个原运行会话的结果仍可读取，归属不变。
- 排空检查允许保留已经无运行、无排队、无子进程且可休眠的 interrupted Agent/Operator；原状态和历史不变，其他执行类型仍受原终态检查约束。
- 在线备份单次遍历目录，最多并行四个根，保留嵌套数据库与符号链接边界。逐库固定读取快照并检查完整性，全部完成才发布；在线备份仍不代表跨库一致性快照。

## 定向验证

只测试改动及直接影响的链路，未运行全量回归：Rust 62 项、Python 64 项、SPA 98 项，以及最终构建的 Operator/Agent E2E 5 项通过。项目页面覆盖 1920、1280、768、390 四种屏宽；Ontology 浏览器与只读 NFS 验收通过。相关 Clippy、格式、SPA 构建一致性及完整发布包校验通过。

| 功能 | 测试入口 |
| --- | --- |
| Operator 首轮、身份、技能、续聊与派生 | [operator_e2e](../../../tests/operator_e2e/main.rs) 的 `operator_session_runs_prompt_to_idle`、`remote_forks_preserve_conversation_type_and_policy_after_restart`、`operator_uses_server_identity_skills_tools_and_resumes_on_host` |
| 普通 Agent 与身份拒绝边界 | 同上，`agent_uses_server_identity_skills_tools_and_resumes_on_host`、`missing_identity_is_rejected_without_a_server_execution_record` |
| 已中断会话的排空边界 | [test_runtimes.py](../../../scripts/platform/maintenance_tests/test_runtimes.py) 的 `test_interrupted_conversation_is_retained_only_after_all_work_has_stopped` 等 4 项 |
| 嵌套数据库、符号链接、损坏库不发布部分结果 | [test_scope.py](../../../scripts/platform/maintenance_tests/backup_files/test_scope.py) 的 `test_online_backup_keeps_nested_databases_and_does_not_follow_symlinks`、`test_corrupt_database_never_publishes_a_partial_online_backup` |

生产实际 Operator 调用完成只读技能目录检查并结束。首次 900 秒观察中 80 个任务完成，但出现 3 次入口采样超时及 30.808 秒调度间隔，该轮判定失败并保留原始记录。后续复验持续 901.84 秒，169 个原生任务全部完成，888 次入口采样无失败；最长受理 4.798 秒、最大受理间隔 10.599 秒、最大调度间隔 10.64 秒。

复验采用同版本已经运行的连续流量验证，按 5 秒间隔提交并检查未完成任务；对原始执行记录独立复算，验收实现与发布源码一致。4 秒入口超时、零失败及 30 秒受理、调度、完成上限均保留。首轮期间观察到磁盘繁忙与落盘等待，复验通过不表示已消除该环境波动。

完整回执与两轮原始证据位于 `/var/tmp/opencoder-deploy-20261010/`：`validation.json`、`live-result.json`、`installed-verification.json`、`project-data-preserved.json`、`observation-attempt-1/`、`observation-attempt-2/`。在线备份覆盖 128 个根、1222 个文件，共 11,597,484,696 字节；停服一致性备份另行封存。所有中间失败记录保留，不以复验结果覆盖。

本次观察结束后，独立的项目 Tag 发布切换到 `rel-e3ca4fab32e4c08d0ac789dcc61b4a2d98c076fd`。核对其 11 个 Operator 实现文件与本次版本一致，公共入口开放且 4 个节点就绪；该后续发布的观察由其发布任务负责，不计入本次 `3e870567` 的验收结果。交接状态见 `subsequent-publication.json`。

相关索引：[会话](../../../agents/session/index.md)、[Worker](../../../agents/worker/index.md)、[Server](../../../agents/server/index.md)、[平台](../../agent-platform/index.md)。
