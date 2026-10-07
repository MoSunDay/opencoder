Commit: c854143bd187656f4d74be6cca0f153176e44a21

# project 模块

项目数据与旧执行链的领域模块。`ProjectStore` 保存项目、专项、TODO 看板字段、Tag 定义与关联及 TODO 指派记录。层级为 TODO → 可选专项 → 可选项目；专项使用 `project_initiatives`，TODO 通过 `initiative_id` 关联。每条指派保存执行类型、名称、ID、同步状态与结论。旧 `ProjectService` 的 plan/execute 路径仍属于本模块，不是看板指派入口。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- [executor/dag_state.rs](../../crates/project/src/executor/dag_state.rs) — `ProjectService` 的 DAG 受理回执固定资源、配置与日期目录；恢复读取原回执，驱动丢失时先清理容器和挂载，再提交终态。看板指派仍走控制面执行入口。
- `crates/store/src/project.rs`、`crates/store/src/project/overview.rs` — 存储接缝与概览投影；进度以看板 `done` 状态计数，悬空归属仍展示为独立专项或未归属 TODO
- `crates/store/src/project/tags.rs` — 纯函数解析可用 Tag、验证选择与按名称整理关联；专项定义覆盖同名项目定义
- `crates/control/src/api/project_links.rs`、`crates/control/src/scheduler/project_assignments.rs` — 关联既有执行并从节点执行结果回写结论；Agent 和 Operator 均读取节点写出的助手正文

## 相关
- [项目模块约定](../../rules/07-project-module-contract.md) — 归属、看板、执行关联、结论回写与进度规则
- [brain](../brain/index.md)、[control](../control/index.md)
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
