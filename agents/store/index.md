Commit: c854143bd187656f4d74be6cca0f153176e44a21

# store 模块

`Store` trait + libsql（WAL）持久化层。细节以代码为准。
接缝：`Arc<dyn Store>`，session/web/todos/control 等全部经此持久化。

## 索引
- `src/lib.rs` — `Store` trait
- `src/libsql_store/` — libsql 实现（WAL）
- [Cargo.toml](../../Cargo.toml) 固定 libsql 上游提交 `0070ff3331cd6d09425b812e1cd3ebe32e1d4206`，避免连接释放时重复关闭 SQLite 句柄；[connection_lifecycle.rs](../../crates/store/tests/connection_lifecycle.rs) 验证连接与派生持有者生命周期，以及最后持有者释放后的 Windows 独占文件访问。
- `src/libsql_store/sessions.rs` — 会话批删（FK 级联）；`node_tasks.rs` — 节点任务与终态清扫
- [libsql_store/messages.rs](../../crates/store/src/libsql_store/messages.rs) — `load_transcript_page` 在 SQL 中投影并按字节切块，保留消息角色、展示原文、合成标记和用量，排除私有 provider 状态；与已有消息块读取共用游标和预算。
- `src/types.rs` — `SessionMeta.kind` 泳道标签（schema v28 起 `sessions.kind TEXT`；创建时定值：`operator`/`agent`/`team`/`dag`/`todos`/`project`/`brain`，存量行为 NULL）
- `src/libsql_store/sessions.rs` 泳道栅栏 — `SessionFilter.kind=None` 的默认清单排除 `kind='operator'`（`s.kind IS NULL OR s.kind <> 'operator'`），精确泳道用 `s.kind = ?`；存量 NULL 行仍走 id 前缀/标题回退
- `src/schedule_types.rs`、`src/libsql_store/schedule.rs` — 调度台账与定义表（schema v26/v27）
- `src/fleet/` — 节点容量/归属/派发回执（`handoff/`），容量领取在 `handoff/capacity.rs`
- `src/fleet/records.rs` — 终态执行索引批删
- `src/libsql_store/brain_layered.rs` + `brain_layered/schema.rs` — v4 分层画布 run/operation/event 投影（additive 建表，不推动 `SCHEMA_VERSION`；`schema_watermark()` 仅供断言，当前值为 33）
- `src/project.rs`、`src/libsql_store/project_links.rs`、`src/sql_store/project_links.rs` — TODO 看板及执行引用；关联只保存能力 ID、执行 ID、类型、名称和创建时间，删除关联不删除节点执行；排序按专项或未归属范围校验，拒绝缺失或外部 TODO，SQLite/MySQL 在事务内提交
- `src/project/tags.rs`、`src/libsql_store/project/tags.rs`、`src/sql_store/catalog/` — Tag 范围解析与关联整理；定义范围固定且所属项目或专项必须存在，SQLite/MySQL 的 Tag 与 TODO 变更原子提交，StarRocks 沿用顺序写入并先验证选择
- `src/libsql_store/schema/catalog.rs`、`src/sql_store/ddl/catalog.rs` — schema v32 的专项、Tag 与 TODO 关联结构；迁移校验专项复制结果，解除旧里程碑的 TODO 归属并移除旧容器，保留执行记录
- [libsql_store/schema/project_links.rs](../../crates/store/src/libsql_store/schema/project_links.rs) — schema v33 在事务内复制、校验并替换 TODO 执行引用表，移除结论与同步状态缓存，保留执行关联和手工看板字段。
- [libsql_store/schema.rs](../../crates/store/src/libsql_store/schema.rs) — 初始化先读取版本，超出当前支持版本时在业务 DDL 前拒绝打开；已有旧版本记录或版本跟踪前的会话表时创建历史项目结构，再运行升级链，支持只有部分业务表的旧数据库
- `src/sql_store/ddl.rs` — MySQL/StarRocks 存量 TODO 排序列升级以 `-1` 标记待回填行；重试只处理标记行，不改动用户已保存的第 0 位
- `src/store/contract/` 与 `src/libsql_store/impl_methods/` 分组组装 Store 接口和实现；`schema/migrations.rs` 保存共享表升级链。旧大脑专属表不再创建，历史数据由经过核准的维护清单单独清理。

- [fleet/report/rows.rs](../../crates/store/src/fleet/report/rows.rs)：批量核验索引不可变字段，仅写入新增或变化行，冲突整批回滚。
- [fleet/handoff/pending.rs](../../crates/store/src/fleet/handoff/pending.rs)：先筛选 prepared 回执，再读取冻结请求，校验 ID 与类型一致性。
- [fleet/handoff/capacity.rs](../../crates/store/src/fleet/handoff/capacity.rs)：复用活跃票据部分索引，保持全机 FIFO 与事务内容量复核。
