Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

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
- `crates/ontology/src/api/graph/` — 有向范围、多跳中心与邻居
- `crates/ontology/src/api/directories.rs` — 目录树与目录创建/移动/更新；
  `validate_membership` 归属校验：目标须为活跃目录，目录节点保持单一
  父级（移动换父传 `allow_existing` 并查环），普通实体可归属多个目录、
  仅拒绝重复归属同一目录
- `crates/ontology/src/api/relationships.rs` — 关系读写；类型带
  `is_directory_membership` 时创建关系先经 `validate_membership` 校验
- `crates/ontology/src/api/entity_creation.rs` — 实体创建
  （`claim_creation` 按 request_id+指纹幂等）；携带 `directory_id` 时
  校验活跃目录并在同一事务补建「目录归属」关系，创建实体即自动入目录
- `crates/ontology/src/database/seeding.rs` — 环境种子：系统「目录」
  实体类型、根目录实体与 `belongs_to` 等系统关系类型（`belongs_to`
  标记 `is_directory_membership`）
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
- [control](../platform/control/index.md) 初始化与角色门禁、就绪和退休生命周期；
  [core](../foundation/core/index.md) 提供配置与平台身份
- `crates/control/src/ontology.rs` — 正文根校验与 `ontology` 只读 NFS 导出
- [Web 页面](../frontend/web/index.md) — 类型与表单位于 `spa/src/ontology/`，图谱用 react-force-graph-3d/Three.js 3D 力导图
- [Server 与备份](../platform/server/index.md) 同时保存数据库和正文；维护升级见
  [发布约定](../../scripts/acceptance/smooth_release/main.py)
- `crates/database/src/lib.rs` — `opencoder-database` 独立读写执行器

## 数据保鲜

- `crates/ontology/src/domain/freshness.rs` — 纯域：级别阶梯（draft→confident，实体 body 内 `data_level`，serde 默认 draft）、策略/运行/评审类型与结算规则；`crates/ontology/src/domain/types.rs` 实体携带 `data_level`
- `crates/ontology/src/database/freshness.rs`、`database/freshness_review.rs` — 三张 JSON 正文表（policies/runs/reviews）读写与单事务评审（重复 409、目录托管保护、回写正文版本、级别 +1）
- `crates/ontology/src/api/freshness.rs` — `/api/ontology/envs/:env/freshness/*` 数据平面路由；写入 `require_manage`
- `crates/ontology/src/freshness.rs` — `AppState::freshness` 操作面；运行仅经此创建与更新
- 不变量与限额（快照不可变、运行单写者、结算一致、摘要/运行项/并发
  上限）以 [功能规则](../../features/platform/ontology/index.md) 的
  「数据保鲜」为准
- 测试：`crates/ontology/tests/freshness.rs`；控制面对应实现在 `crates/control/src/ontology_freshness/`（触发路由、派发编排、重启回收）

## 相关
- [Ontology 能力](../../features/platform/ontology/index.md)
