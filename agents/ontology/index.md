Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# ontology 模块

`opencoder-ontology` 提供通用类型、实体、关系、图谱切面、属性正文和向量的独立存储及 HTTP 接口。业务 Server 持有数据库；只读资源服务持有正文导出。

## 索引

- [http.rs](../../crates/ontology/src/http.rs)、[api/mod.rs](../../crates/ontology/src/api/mod.rs) — `AppState::open/ready/drained` 与 `/api/ontology` 路由，复用平台 `Identity`；域读取开放给 User/Root，写入仅 Admin。
- [database/schema.rs](../../crates/ontology/src/database/schema.rs)、[database/store.rs](../../crates/ontology/src/database/store.rs) — 独立 `<server-data>/ontology.db`，SQLite WAL；对象按 `env_num` 隔离，用唯一键与索引约束类型、属性和切面。`ontology_schema_version` 在首次初始化时固定实际正文根，供备份定位；之后重启必须使用同一规范化路径，空库也不能重绑。路径变化使启动失败，原绑定、当前正文和历史引用保持原值。
- [mutations.rs](../../crates/ontology/src/mutations.rs) — 每个 HTTP 请求独立连接与事务；写请求成功才提交，错误回滚域数据、审计、正文当前版本与幂等记录。已接纳写入在请求断开后继续完成，退休等待活动写入归零。
- [database/model](../../crates/ontology/src/database/model/mod.rs)、[domain/validation.rs](../../crates/ontology/src/domain/validation.rs) — 类型、属性、Action 配置、关系、切面和 revision 校验；Action 仅保存定义，没有执行入口。
- [text_store.rs](../../crates/ontology/src/text_store.rs)、[database/text_revision.rs](../../crates/ontology/src/database/text_revision.rs) — UUID 与 revision 组成的不可变正文路径；独占创建、文件和祖先目录同步、路径边界及 SHA256/字节数核验，单份正文最多 4 MiB。NFS 引用读取内容校验失败返回冲突，不能覆盖历史。
- [api/graph](../../crates/ontology/src/api/graph/mod.rs)、[api/directories.rs](../../crates/ontology/src/api/directories.rs) — 有向上下游范围、多个中心、关系筛选、跨类型邻居与固定切面；系统目录约束单父级并拒绝环。
- [database/vectors.rs](../../crates/ontology/src/database/vectors.rs)、[api/vectors.rs](../../crates/ontology/src/api/vectors.rs) — 2048 维有限非零向量，余弦搜索按环境和属性隔离。

## 接缝

- [control](../control/index.md) 初始化与角色门禁、就绪和退休生命周期；[core](../core/index.md) 提供配置与平台身份。
- [control/ontology.rs](../../crates/control/src/ontology.rs) 校验正文根、管理名为 `ontology` 的第四个只读 NFS 导出；正常切换保留独立资源进程和端口。
- [Web 页面](../web/index.md) 通过平台请求与环境上下文调用接口，类型及表单位于 `spa/src/ontology/`，图谱使用 G6。
- [Server 与备份](../server/index.md) 同时保存数据库和正文；数据契约为 3，旧格式升级进入维护流程。

## 相关

- [Ontology 能力](../../features/ontology/index.md)
- [正文根绑定测试](../../crates/ontology/tests/storage_root.rs)、[HTTP 存储测试](../../crates/ontology/tests/storage.rs)、[图谱与权限测试](../../crates/ontology/tests/graph.rs)、[控制面与 NFS 测试](../../crates/control/tests/ontology.rs)
