Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# ontology 模块

类型、实体、关系、图谱切面、属性正文与向量的独立存储及 HTTP 接口；
版本 Server 持有数据库，只读资源服务持有正文导出。

## 索引
- `crates/ontology/src/http.rs`、`api/mod.rs` — `/api/ontology` 路由与
  `AppState::open/ready/drained` 生命周期
- `crates/ontology/src/database/schema/`、`database/store.rs` — 独立
  `ontology.db`；Rust 与发布预检共用 SQL 定义，逐项比对结构
- `crates/ontology/src/mutations.rs` — 每请求独占连接与事务
- `crates/ontology/src/database/model/`、`domain/validation.rs` — 类型、
  属性、Action 配置、关系、切面与 revision 校验
- `crates/ontology/src/text_store.rs`、`database/text_revision.rs` —
  UUID+revision 不可变正文路径
- `crates/ontology/src/api/graph/`、`api/directories.rs` — 有向范围、
  多跳中心、邻居与系统目录
- `crates/ontology/src/database/vectors.rs`、`api/vectors.rs` — 向量读写
  非零向量与余弦搜索
- `crates/ontology/src/catalog.rs`、`database/catalog/` — 控制面能力
  快照的目录同步

## 测试
- `crates/ontology/tests/storage_root.rs` — 正文根绑定回归
- `crates/ontology/tests/storage.rs` — HTTP 存储回归
- `crates/ontology/tests/graph.rs` — 图谱与权限回归
- `crates/ontology/tests/entity_lookup.rs` — 实体查询范围回归
- `crates/control/tests/ontology.rs` — 控制面与 NFS 导出回归

## 接缝
- [control](../control/index.md) 初始化与角色门禁、就绪和退休生命周期；
  [core](../core/index.md) 提供配置与平台身份
- `crates/control/src/ontology.rs` — 正文根校验与 `ontology` 只读 NFS 导出
- [Web 页面](../web/index.md) — 类型与表单位于 `spa/src/ontology/`，图谱用 G6
- [Server 与备份](../server/index.md) 同时保存数据库和正文；维护升级见
  [发布约定](../../docs/smooth-release.md)
- `crates/database/src/lib.rs` — `opencoder-database` 独立读写执行器

## 数据保鲜

- `crates/ontology/src/domain/freshness.rs` — 纯域：级别阶梯（draft→confident，实体 body 内 `data_level`，serde 默认 draft）、策略/运行/评审类型与结算规则；`crates/ontology/src/domain/types.rs` 实体携带 `data_level`
- `crates/ontology/src/database/freshness.rs`、`database/freshness_review.rs` — 三张 JSON 正文表（policies/runs/reviews）读写与单事务评审（重复 409、目录托管保护、回写正文版本、级别 +1）
- `crates/ontology/src/api/freshness.rs` — `/api/ontology/envs/:env/freshness/*` 数据平面路由；写入 `require_manage`
- `crates/ontology/src/freshness.rs` — `AppState::freshness` 操作面；运行仅经此创建与更新
- 不变量：运行快照创建后不可变（scope 锁定）；运行写入单写者（控制面）；结算规则与数据平面 `settle` 一致；候选摘要 ≤ 8000 字符、运行项 ≤ 500、并发 1..30
- 测试：`crates/ontology/tests/freshness.rs`；控制面对应实现在 `crates/control/src/ontology_freshness/`（触发路由、派发编排、重启回收）

## 相关
- [Ontology 能力](../../features/ontology/index.md)
