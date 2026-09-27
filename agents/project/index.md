Commit: 6ab6ec63595b45b7440f047d768fff7108a6ab04

# project 模块

项目数据与旧执行链的领域模块。`ProjectStore` 保存项目、里程碑、专项、TODO 和 TODO→执行 ID 关联。里程碑与专项是同级分组，共用 `project_milestones` 表，以 `kind` 区分，各自可不关联项目；TODO 可归属任一分组或不分组。Server 项目工作台通过执行索引读取类型、名称、状态，复用各能力原生执行面。旧 `ProjectService` 的 plan/execute 路径仍属于本模块，不是新工作台的提交入口。

## 索引
- `crates/project/src/` — 旧项目执行入口与领域服务
- `crates/store/src/project.rs`、`crates/store/src/project/overview.rs` — 分组存储接缝与概览投影
- `crates/control/src/api/project_links.rs` — TODO 执行关联 API

## 相关
- [brain](../brain/index.md)、[control](../control/index.md)
