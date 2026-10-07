Commit: c854143bd187656f4d74be6cca0f153176e44a21

# project 模块

项目数据与旧执行链的领域模块。`ProjectStore` 保存项目、专项、TODO 看板字段、Tag 定义与关联及 TODO 指派记录。层级为 TODO → 可选专项 → 可选项目；专项使用 `project_initiatives`，TODO 通过 `initiative_id` 关联。每条关联只保存 TODO ID、执行 ID、能力 ID、类型、名称和创建时间；会话、状态与结论由所属执行节点提供。旧 `ProjectService` 的 plan/execute 路径仍属于本模块，不是看板指派入口。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- [executor/dag_state.rs](../../crates/project/src/executor/dag_state.rs) — `ProjectService` 的 DAG 受理回执固定资源、配置与日期目录；恢复读取原回执，驱动丢失时先清理容器和挂载，再提交终态。看板指派仍走控制面执行入口。
- `crates/store/src/project.rs`、`crates/store/src/project/overview.rs` — 存储接缝与概览投影；进度以看板 `done` 状态计数，悬空归属仍展示为独立专项或未归属 TODO
- `crates/store/src/project/tags.rs` — 纯函数解析可用 Tag、验证选择与按名称整理关联；专项定义覆盖同名项目定义
- [project_links.rs](../../crates/control/src/api/project_links.rs)、[project_links/](../../crates/control/src/api/project_links/) — 关联既有执行并核验能力身份；派发适配器通过既有 Fleet 回执固定能力和请求，复用原生执行入口，无项目执行循环。
- [executions/results/](../../crates/control/src/api/executions/results/) — 按需从所属节点读取当前结论；离线返回错误，DAG 产物由步骤明细读取。

## 相关
- [brain](../brain/index.md)、[control](../control/index.md)
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
