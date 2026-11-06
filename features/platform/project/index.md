Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# 项目工作台

项目/子项/TODO 的目标与看板工作台，关联既有执行与任务文件；本卡只做指路。

## 索引

- [project 域卡](../../../agents/workbench/project/index.md) — 项目域完整代码索引
- 字段口径：`initiative`、`detail_md`、`current_summary`、
  `summary_updated_at`
- [crates/project/src/service.rs](../../../crates/project/src/service.rs)、
  [context.rs](../../../crates/project/src/context.rs) — 领域服务与
  `project todos context` 证据读取
- [crates/store/src/project/](../../../crates/store/src/project) — 存储投影与
  看板校验（summary/tags/people/acceptance/comments）
- [project_links.rs](../../../crates/control/src/api/project_links.rs)、
  [project_links/](../../../crates/control/src/api/project_links) — 关联既有执行
  与派发
- [project_files/](../../../crates/control/src/project_files) —
  任务文件上传与只读 NFS 导出
- [crates/web/src/api_project.rs](../../../crates/web/src/api_project.rs)、
  [api_project/](../../../crates/web/src/api_project) — 工作台 API
- [crates/ctl/src/cmd/project.rs](../../../crates/ctl/src/cmd/project.rs) —
  `opencoder-cli project` 命令
- [项目 CLI](../../../docs/project-cli.md)、
  [任务文件](../../../docs/project-files.md)、
  [概述与快照](../../../docs/project-summaries.md)
- [项目模块约定](../../../rules/07-project-module-contract.md)、
  [opencoder-projects skill](../../../skills/opencoder-projects/SKILL.md)

## 相关

- [项目逻辑](../../../agents/workbench/project/index.md)、
  [项目约定](../../../rules/07-project-module-contract.md)、
  [Control](../../../agents/platform/control/index.md)、[TODO 工作台](../todos/index.md)
