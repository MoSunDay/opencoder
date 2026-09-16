Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# project 模块

项目跟踪：goal→milestone→todo。

## 索引
- `crates/project/src/` — 领域模型与执行入口

## 接缝
- `ProjectStore` 接缝；运行复用 session/todos 执行面。
- 历史 Brain/Playbook executor 字段仍可读取，但解析和执行入口返回迁移错误。新图计划经独立 `/api/brain/runs` 契约执行；Project 不再隐式生成决策树或启动旧 Playbook。
- 相关：[brain](../brain/index.md)、[control](../control/index.md)。
