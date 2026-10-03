Commit: 347a6bdfee28570f5c5cf9e2e1891d563cdf1bb7

# 固定源码的平台回归与仓库范围复核

仓库说明只记录 OpenCoder 自身能力和契约，外部系统原稿独立保存。运行实现、界面源码及产物保持已提交基线内容。

平台验证使用固定源码、独立构建产物、临时数据库和私有挂载空间。开发工作区后续改动按自己的源码内容验证，不能沿用提交号相同的旧结果。

## 测试覆盖

| 功能 | 测试或校验入口 | 文件 |
| --- | --- | --- |
| 内置技能及用户资源边界 | `seed_in_writes_all_packs_on_fresh_dir`、`seed_builtin_skills_backs_up_then_overwrites_user_edits` | `crates/core/tests/skill_contract/seeding.rs` |
| 原生步骤、取消超时及输出上限 | `runc_step_smoke`、`cancellation_and_timeout_remove_running_containers`、`stdout_overflow_fails_and_removes_container` | `crates/dag-runtime/src/sandbox/runc.rs` |
| 只读 NFS 与离线恢复 | `manual_mount_e2e`、`kernel_plain_readdir_continues_across_exporter_replacement`、`readonly_nfs_node_snapshots_and_offline_followup` | `crates/agents/src/serve/tests.rs`、`crates/agents/tests/nfs_pagination.rs`、`crates/worker/tests/nfs_mount.rs` |
| 计划画布、并行返回与执行明细 | `schema_seven_canvas_parallel_return_and_execution_detail` | `crates/worker/tests/brain_browser.rs` |
| 界面及产物一致性 | SPA 全量测试、严格重建比较 | `crates/web/spa`、`scripts/check-spa-drift.sh` |

- 固定源码全量回归：5593 项通过、零失败；7 项真实环境用例另行全部通过。
- 工作区格式、全目标 Clippy 零警告及二进制与运行器构建通过；成套产物的构建信息一致。
- SPA 928 项通过，构建产物严格重建无差异。
- 已提交源码 3061 份文件和开发工作区非生成文件的范围复核通过；原始输出及源码对照保存在 `/data00/opencoder-safety/20261003-closure/`。

[Core](../../../agents/core/index.md) · [DAG 执行约定](../../../rules/04-dag-execution-contract.md) · [UI 验收约定](../../../rules/05-ui-acceptance.md)
