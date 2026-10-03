Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# 配置覆盖、安装文件与记忆模板保护

局部项目配置现在保留全局未指定的 Codex 启动信息和 NFS 导出字段。运行配置保留其他 profile，同名 profile 按完整版本替换；加载结束后检查合并结果的私有配置容量，避免单文件均有效但派发结果超限。

Windows Computer-use 安装器在任何本地写入前检查完整下载清单，解包前检查全部 ZIP/tar 成员，拒绝越界、路径别名、重解析点和归档链接。下载先写临时文件，校验成功才替换已有文件。内置记忆模板默认使用 `repo-memory.md`，防止覆盖 Windows 上的 `AGENTS.md`。

## 测试覆盖

| 功能 | 测试或校验入口 | 文件 |
| --- | --- | --- |
| 局部 Codex 配置与冻结 Operator | `project_codex_overlay_preserves_global_launch_settings_and_frozen_operator` | `crates/core/tests/config_overlay/main.rs` |
| profile 保留及完整版本替换 | `project_profiles_preserve_other_names_and_replace_whole_revisions` | `crates/core/tests/config_overlay/main.rs` |
| 显式字段及 null 清除 | `codex_patch_changes_only_explicit_fields_and_null_clears_settings` | `crates/core/tests/config_overlay/main.rs` |
| NFS 局部配置保留原导出字段 | `partial_nfs_overlays_preserve_enabled_host_and_read_only_global_settings` | `crates/core/tests/config_overlay/main.rs` |
| 合并容量限制与错误脱敏 | `resolved_private_overlays_reject_combined_dispatch_budgets_without_exposing_values` | `crates/core/tests/config_overlay/main.rs` |
| 默认索引与仓库指令文件分离 | `seeded_memory_template_uses_a_distinct_index_and_preserves_repository_instructions` | `crates/core/tests/skill_contract/seeding.rs` |
| Windows 清单、归档与文件替换 | 36 项原生 PowerShell 检查 | `tools/computer-use/ops/windows/artifact-tests.ps1` |
| Windows 与 Unix 技能目录的用户根和路径语义 | `skills_dir_points_at_global_home`、`skills_dir_without_home_is_none_or_absolute_never_cwd` | `crates/core/tests/skill_contract/discovery.rs` |

- Rust 全量回归：5,677 项通过、零失败；7 项既有环境用例由正式进程验收执行。
- 格式检查、全工作区构建及全目标 Clippy 通过；SPA 966 项、Computer-use 63 项、发布控制器 127 项、维护验收脚本 18 项通过。
- 回归使用配套程序和官方方式准备的三个运行器。正式包、跨平台 CI、真实磁盘上的 UI、runc 与发布恢复证据须对应同一固定候选，不沿用其他源码版本的验收结论。

[Core](../../../agents/core/index.md) · [Windows](../../windows/index.md) · [Computer use](../../computer-use/index.md)
