Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# project 模块

项目数据与旧执行链的领域模块；层级为 TODO → 可选专项 → 可选项目，
看板指派走控制面执行入口。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- `crates/project/src/executor/dag_state.rs` — `ProjectService` 的 DAG
  受理回执与恢复
- `crates/store/src/project.rs` — `ProjectStore` 存储接缝与执行持久化
- `crates/store/src/project/overview.rs` — 概览快照与纯进度投影
- `crates/store/src/project/summary.rs` — 用户维护概述与版本数上限
  `SUMMARY_HISTORY_LIMIT`
- `crates/store/src/project/tags.rs` — Tag 范围解析纯函数
- `crates/store/src/project/people.rs` — 人员归属与关注人字段规范化
- `crates/store/src/project/acceptance.rs` — 看板完成前提纯校验
- `crates/store/src/project/comments.rs` — TODO 评论数据与幂等追加
- `crates/control/src/api/project_links.rs`、`project_links/` — 关联既有
  执行与派发适配
- `crates/control/src/api/executions/results/` — 按需读取执行当前结论
- `crates/control/src/project_files/mod.rs` — Server 上传任务文件与只读
  NFS 导出，见 [任务文件](../../../docs/project-files.md)

## 相关
- [项目模块约定](../../../rules/07-project-module-contract.md) — 归属、看板、执行关联、实时结果读取与进度规则
- [brain](../../orchestration/brain/index.md)、[control](../../platform/control/index.md)
- [dag-runtime](../../workflow/dag-runtime/index.md)、[执行约定](../../../rules/04-dag-execution-contract.md)
