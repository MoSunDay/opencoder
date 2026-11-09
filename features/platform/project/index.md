Commit: 4a6a4181c3cce8359e1668735b4e69695a0daf2a

# 项目工作台

项目/子项/TODO 的目标与看板工作台，关联既有执行与任务文件；本卡只做指路。

## 索引

- [project 域卡](../../../agents/workbench/project/index.md) — 项目域完整代码索引
  （领域服务、store 投影、project_links 与任务文件）
- 字段口径：`initiative`、`detail_md`、`current_summary`、
  `summary_updated_at`
- [crates/web/src/api_project.rs](../../../crates/web/src/api_project.rs)、
  [api_project/summary.rs](../../../crates/web/src/api_project/summary.rs) —
  工作台 API 与概述快照
- [crates/ctl/src/cmd/project.rs](../../../crates/ctl/src/cmd/project.rs) —
  `opencoder-cli project` 命令
- [opencoder-projects skill](../../../skills/opencoder-projects/SKILL.md)

## 相关

- [项目约定](../../../rules/07-project-module-contract.md)、
  [Control](../../../agents/platform/control/index.md)、[TODO 工作台](../todos/index.md)
