Commit: f90df21319a94ee4e7d2b560c32a25a61dff68ad

# ontology 模块

`opencoder-ontology` 提供通用类型、实体、关系、图谱切面、属性正文和向量的独立存储及 HTTP 接口。版本 Server 持有数据库；只读资源服务持有正文导出。

## 索引

- [http.rs](../../crates/ontology/src/http.rs)、[api/mod.rs](../../crates/ontology/src/api/mod.rs) — `AppState::open/ready/drained` 与 `/api/ontology` 路由，复用平台 `Identity`；域读取开放给 User/Root，写入仅 Admin。
- [database/schema/](../../crates/ontology/src/database/schema/mod.rs)、[database/store.rs](../../crates/ontology/src/database/store.rs) — 独立 `<server-data>/ontology.db`，Turso WAL；对象按 `env_num` 隔离，用唯一键与索引约束类型、属性和切面。正常打开只校验结构；空库初始化与维护迁移在同一事务中完成结构和系统数据准备。Rust 与发布预检共用 SQL 定义，逐项比对列、主键、约束和索引；缺失对象要求维护，冲突对象拒绝覆盖。`ontology_schema_version` 固定实际正文根，重启和维护均不能重绑。
- [mutations.rs](../../crates/ontology/src/mutations.rs) — 每个 HTTP 请求独占连接池中的连接与事务；写请求成功才提交，错误回滚域数据、审计、正文当前版本与幂等记录。已接纳写入在请求断开后继续完成，退休等待活动写入归零。
- [database/model](../../crates/ontology/src/database/model/mod.rs)、[domain/validation.rs](../../crates/ontology/src/domain/validation.rs) — 类型、属性、Action 配置、关系、切面和 revision 校验；Action 仅保存定义，没有执行入口。
- [text_store.rs](../../crates/ontology/src/text_store.rs)、[database/text_revision.rs](../../crates/ontology/src/database/text_revision.rs) — UUID 与 revision 组成的不可变正文路径；独占创建、文件和祖先目录同步、路径边界及 SHA256/字节数核验，单份正文最多 4 MiB。NFS 引用读取内容校验失败返回冲突，不能覆盖历史。
- [api/graph](../../crates/ontology/src/api/graph/mod.rs)、[api/directories.rs](../../crates/ontology/src/api/directories.rs) — 有向上下游范围、多个中心、关系筛选、跨类型邻居与固定切面；系统目录约束单父级并拒绝环。
- [database/vectors.rs](../../crates/ontology/src/database/vectors.rs)、[api/vectors.rs](../../crates/ontology/src/api/vectors.rs) — 2048 维有限非零向量，余弦搜索按环境和属性隔离。

## 接缝

- [catalog.rs](../../crates/ontology/src/catalog.rs) 与 [database/catalog](../../crates/ontology/src/database/catalog/mod.rs) 接受控制面提供的完整能力快照，在独立事务中维护稳定实体、结构化描述、目录及依赖。`catalog_entries` 保存来源快照，`catalog_objects` 记录生成对象和内容摘要，`catalog_state` 保存进度与错误；读写均参与退休等待。
- HTTP 写入先检查生成对象归属；原生定义通过来源模块修改，补充说明及自定义数据保持可写。来源与补充正文仅首次初始化，定义变化更新结构化属性，避免回滚重试覆盖不可变文件。无差异批次仅更新同步状态。

- [control](../control/index.md) 初始化与角色门禁、就绪和退休生命周期；[core](../core/index.md) 提供配置与平台身份。
- [control/ontology.rs](../../crates/control/src/ontology.rs) 校验正文根、管理名为 `ontology` 的第四个只读 NFS 导出；正常切换保留独立资源进程和端口。
- [Web 页面](../web/index.md) 通过平台请求调用当前 Server 的默认 `debug` 数据空间，页面不提供环境选择与管理；类型及表单位于 `spa/src/ontology/`，图谱使用 G6。
- [Server 与备份](../server/index.md) 同时保存数据库和正文；数据格式兼容性与维护升级入口见 [发布约定](../../docs/smooth-release.md)。

## 相关

- [Ontology 能力](../../features/ontology/index.md)
- [正文根绑定测试](../../crates/ontology/tests/storage_root.rs)、[HTTP 存储测试](../../crates/ontology/tests/storage.rs)、[图谱与权限测试](../../crates/ontology/tests/graph.rs)、[控制面与 NFS 测试](../../crates/control/tests/ontology.rs)

## 实体查询范围

结构化属性与文本版本按环境、实体 ID 使用已有索引读取；文本版本继续按属性筛选并倒序返回。普通实体加入目录只检查自身归属，目录实体仍检查父级链和环。对应回归见 `crates/ontology/tests/entity_lookup.rs`。

数据库通过 [opencoder-database](../../crates/database/src/lib.rs) 的独立读、写执行器运行；请求中的任务局部连接贯穿处理和事务收尾。退休计数覆盖提交或回滚完成，客户端断开不取消已接纳写入。
