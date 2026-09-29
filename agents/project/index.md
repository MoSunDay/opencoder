Commit: 51cb1e361e0774effa9f4eb38e93cc75e432b068

# project 模块

项目数据与旧执行链的领域模块。`ProjectStore` 保存项目、里程碑、专项、TODO 看板字段及 TODO 指派记录。里程碑与专项是同级分组，共用 `project_milestones` 表，以 `kind` 区分，各自可不关联项目；TODO 可归属任一分组或不分组。每条指派保存执行类型、名称、ID、同步状态与结论。旧 `ProjectService` 的 plan/execute 路径仍属于本模块，不是看板指派入口。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- `crates/store/src/project.rs`、`crates/store/src/project/overview.rs` — 分组存储接缝与概览投影
- `crates/control/src/api/project_links.rs`、`crates/control/src/scheduler/project_assignments.rs` — 关联既有执行并从节点执行结果回写结论；Agent 和 Operator 均读取节点写出的助手正文

## 相关
- [brain](../brain/index.md)、[control](../control/index.md)
