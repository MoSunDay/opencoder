Commit: fd9fdc34a732e0fab72d1a664792c7654528ab63

# 技能测试拆分与业务描述清理

技能契约按发现、种子写入、规划和工作流拆分，原有 28 个用例完整保留。首次安装检查准确的内置技能与资源集合，用户自定义资源保持原样。

已退出业务的说明、配置注释和旧示例引用移到仓库外；通用规划、注册 CLI、模型路由与编译说明保持完整。SPA 产物检查独立复制包内资源并严格比较构建结果。运行实现、UI 源码和产物保留最新基线的内容，生产 Rust 的修改仅为三处注释。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 首次安装只有准确的内置技能 | `seed_in_writes_all_packs_on_fresh_dir` | `crates/core/tests/skill_contract/seeding.rs` |
| 更新备份内置修改，用户资源原样保留 | `seed_builtin_skills_backs_up_then_overwrites_user_edits` | `crates/core/tests/skill_contract/seeding.rs` |
| 规划资源清单准确 | `seeded_task_plan_skill_requires_launch_closure_contract` | `crates/core/tests/skill_contract/planning.rs` |

- 合并基线的全量结果：5593 Rust 用例通过、7 项原有手动用例另行通过；928 项 SPA 用例、全站 UI、真实 runc 与发布演练通过，见同日 `ui-native-dag-closure.md`。
- 本轮补充验证：技能契约 28 项通过；Core 全目标 Clippy 零警告，工作区格式、SPA 产物严格一致性通过。
- 本轮也完成较大固定源码版本的全量回归：5634 Rust、949 SPA 用例通过；该版本的其他通用功能仍保留在共享工作区，未并入本提交。
- 新基线上的 3060 份文件扫描无业务残留；201 份迁移文件摘要一致。
- 原始输出与源码对照位于 `/data00/opencoder-safety/20261001-103221/commit-cleanup-*.log`、`commit-cleanup-preparation.json`、`commit-cleanup-overlay-rebase.json`。

[Core](../../../agents/core/index.md) · [全站与原生 DAG 验证](ui-native-dag-closure.md)
