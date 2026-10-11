Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# project 模块

项目数据与旧执行链的领域模块。`ProjectStore` 保存项目、专项、TODO 看板字段、Tag 定义与关联及 TODO 指派记录。层级为 TODO → 可选专项 → 可选项目；专项使用 `project_initiatives`，TODO 通过 `initiative_id` 关联。每条关联只保存 TODO ID、执行 ID、能力 ID、类型、名称和创建时间；会话、状态与结论由所属执行节点提供。旧 `ProjectService` 的 plan/execute 路径仍属于本模块，不是看板指派入口。

- 专项没有自身状态；`ProjectInitiativeRecord/Patch` 不含 `status`，完成比例仅按 TODO `board_status` 计算。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- [executor/dag_state.rs](../../crates/project/src/executor/dag_state.rs) — `ProjectService` 的 DAG 受理回执固定资源、配置与日期目录；恢复读取原回执，驱动丢失时先清理容器和挂载，再提交终态。看板指派仍走控制面执行入口。
- `crates/store/src/project.rs`、`crates/store/src/project/overview.rs` — 存储接缝与概览投影；进度以看板 `done` 状态计数，悬空归属仍展示为独立专项或未归属 TODO
- [project/summary.rs](../../crates/store/src/project/summary.rs) — 项目与专项的用户维护概述及独立更新时间；概述不参与进度与验收，不作为原生执行结果缓存。每个对象保留最近 10 个版本，回滚新增版本；证据收集和版本入口见 [ctl](../ctl/index.md)。
- `crates/store/src/project/tags.rs` — 纯函数解析可用 Tag、验证选择与按名称整理关联；同名定义按专项、项目、全局优先，未归属 TODO 仅可选全局定义
- [project/people.rs](../../crates/store/src/project/people.rs)、[存储人员关系](../../crates/store/src/libsql_store/project/people.rs) — 三层独立人员字段、关注人集合规范化及 PATCH 清空语义；对象、人员与 Tag 在同一事务提交。
- [project_links.rs](../../crates/control/src/api/project_links.rs)、[project_links/](../../crates/control/src/api/project_links/) — 关联既有执行并核验能力身份；派发适配器通过既有 Fleet 回执固定能力和请求，复用原生执行入口，无项目执行循环。
- [executions/results/](../../crates/control/src/api/executions/results/) — 按需从所属节点读取当前结论；离线返回错误，DAG 产物由步骤明细读取。
- [完成前提](../../crates/store/src/project/acceptance.rs)、[事务核验](../../crates/store/src/libsql_store/project/acceptance.rs) — 验证读取时的任务、能力、执行和依赖仍有效，再修改看板；前提过期由 API 返回 409。

## 相关
- [项目模块约定](../../rules/07-project-module-contract.md) — 归属、看板、执行关联、实时结果读取与进度规则
- [brain](../brain/index.md)、[control](../control/index.md)
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
