Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# project 模块

项目数据与旧执行链的领域模块。`ProjectStore` 保存项目、专项、TODO 看板字段、Tag 定义与关联、TODO 指派记录及独立评论。层级为 TODO → 可选专项 → 可选项目；专项使用 `project_initiatives`，TODO 通过 `initiative_id` 关联。每条关联只保存 TODO ID、执行 ID、能力 ID、类型、名称和创建时间；会话、状态与结论由所属执行节点提供。旧 `ProjectService` 的 plan/execute 路径仍属于本模块，不是看板指派入口。

- 专项没有自身状态；`ProjectInitiativeRecord/Patch` 不含 `status`，完成比例仅按 TODO `board_status` 计算。

网页显示“子项”对应专项 `initiative`，“目标”对应 `detail_md`，“现状”对应 `current_summary`。折叠只在 Web 组件中维护，字段、归属与存储读取沿用现有接口；前端结构见 [web 模块](../web/index.md)。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- [executor/dag_state.rs](../../crates/project/src/executor/dag_state.rs) — `ProjectService` 的 DAG 受理回执固定资源、配置与日期目录；恢复读取原回执，驱动丢失时先清理容器和挂载，再提交终态。看板指派仍走控制面执行入口。
- `crates/store/src/project.rs`、`crates/store/src/project/overview.rs` — 存储接缝与概览投影；进度以看板 `done` 状态计数，悬空归属仍展示为独立专项或未归属 TODO
- [project/summary.rs](../../crates/store/src/project/summary.rs) — 项目与专项的用户维护概述及独立更新时间；概述不参与进度与验收，不作为原生执行结果缓存。每个对象保留最近 10 个版本，回滚新增版本；证据收集和版本入口见 [ctl](../ctl/index.md)。
- `crates/store/src/project/tags.rs` — 纯函数解析可用 Tag、验证选择与按名称整理关联；同名定义按专项、项目、全局优先，未归属 TODO 仅可选全局定义
- [project/people.rs](../../crates/store/src/project/people.rs)、[存储人员关系](../../crates/store/src/turso_store/projects/project/people.rs) — 三层独立人员字段、关注人集合规范化及 PATCH 清空语义；对象、人员与 Tag 在同一事务提交。
- [project_links.rs](../../crates/control/src/api/project_links.rs)、[project_links/](../../crates/control/src/api/project_links/) — 关联既有执行并核验能力身份；派发适配器通过既有 Fleet 回执固定能力和请求，复用原生执行入口，无项目执行循环。
- [executions/results/](../../crates/control/src/api/executions/results/) — 按需从所属节点读取当前结论；离线返回错误，DAG 产物由步骤明细读取。
- [完成前提](../../crates/store/src/project/acceptance.rs)、[事务核验](../../crates/store/src/turso_store/projects/project/acceptance.rs) — 验证读取时的任务、能力、执行和依赖仍有效，再修改看板；前提过期由 API 返回 409。

- [评论数据](../../crates/store/src/project/comments.rs)、[评论存储](../../crates/store/src/turso_store/projects/project/comments.rs) — schema 39 使用递增序号分页，按 TODO、提交用户与请求 ID 保证事务追加幂等；同时核验任务及执行关联，更新任务时间。[评论修改](../../crates/store/src/turso_store/projects/project/comments/mutations.rs) 在同一写事务中核验 TODO、提交用户权限和版本，更新正文或删除记录；独立状态表保存版本、原追加正文及删除标记，防止原追加请求恢复已删除评论。新增状态表不改写已有任务与认证数据。正文与署名快照不引用用户、能力的可变名称；TODO 删除触发同事务清理，评论不作为执行结果缓存。

- [project_files](../../crates/control/src/project_files/mod.rs) 管理 Server 上主动上传的任务文件，使用创建日期、TODO ID 和相对路径定位；独立 NFS 只读导出。目录绑定、文件锁、原子发布及备份协调与数据库任务删除配合，原生执行记录仍由执行节点持有。配置和接口见 [任务文件](../../docs/project-files.md)。

## 相关
- [项目模块约定](../../rules/07-project-module-contract.md) — 归属、看板、执行关联、实时结果读取与进度规则
- [brain](../brain/index.md)、[control](../control/index.md)
- [dag-runtime](../dag-runtime/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
