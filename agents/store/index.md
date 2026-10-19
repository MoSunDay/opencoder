Commit: 8eef2858b435e869c541d6ae8222d9d66687b839

# store 模块

`Store` trait + Turso（WAL）持久化层。细节以代码为准。
接缝：`Arc<dyn Store>`，session/web/todos/control 等全部经此持久化。
项目接口 `ProjectStore` 由同一 `TursoStore` 实现；独立 Web 在 [serve](../../crates/web/src/lib.rs) 中复用同一实例，项目后端没有独立配置或工厂。

- [专项迁移](../../crates/store/src/turso_store/bootstrap/schema/catalog.rs)：schema v38 移除 `project_initiatives.status`；保留 TODO、归属、排序、概述、Tag 和人员，事务失败完整回滚。

## 索引
- [schema/access.rs](../../crates/store/src/turso_store/bootstrap/schema/access.rs)、[users/tokens.rs](../../crates/store/src/turso_store/identity/users/tokens.rs) — schema v35 分离用户身份与独立 Token；鉴权读取用户当前角色，逐 Token 校验到期与撤销状态，升级保留已有凭据摘要。
- `src/lib.rs` — `Store` trait
- `src/turso_store/` — Turso 实现（WAL）
- [Cargo.toml](../../Cargo.toml) 固定 Turso 0.8.3；[database](../../crates/database/src/lib.rs) 统一执行 Store、Fleet、TsRegistry 和 Ontology 的数据库操作，应用线程只等待异步结果。
- `src/turso_store/conversation/sessions.rs` — 会话批删（FK 级联）；`node_tasks.rs` — 节点任务与终态清扫
- [turso_store/conversation/messages.rs](../../crates/store/src/turso_store/conversation/messages.rs) — `load_transcript_page` 在 SQL 中投影并按字节切块，保留消息角色、展示原文、合成标记和用量，排除私有 provider 状态；与已有消息块读取共用游标和预算。
- `src/types.rs` — `SessionMeta.kind` 泳道标签（schema v28 起 `sessions.kind TEXT`；创建时定值：`operator`/`agent`/`team`/`dag`/`todos`/`project`/`brain`，存量行为 NULL）
- `src/turso_store/conversation/sessions.rs` 泳道栅栏 — `SessionFilter.kind=None` 的默认清单排除 `kind='operator'`（`s.kind IS NULL OR s.kind <> 'operator'`），精确泳道用 `s.kind = ?`；存量 NULL 行仍走 id 前缀/标题回退
- `src/schedule_types.rs`、`src/turso_store/execution/schedule.rs` — 调度台账与定义表（schema v26/v27）
- `src/fleet/` — 节点容量/归属/派发回执（`handoff/`），容量领取在 `handoff/capacity.rs`
- [fleet/handoff/acceptance.rs](../../crates/store/src/fleet/handoff/acceptance.rs)、[receipts.rs](../../crates/store/src/fleet/handoff/receipts.rs) — Agent/Operator 准备派发只保存冻结请求与回执，远端持久接收后才发布执行索引；确认和节点上报都核对冻结归属，迟到确认不覆盖较新的状态。其他执行种类保留准备阶段索引。
- `src/fleet/records.rs` — 终态执行索引批删
- `src/turso_store/execution/brain_layered.rs` + `brain_layered/schema.rs` — v4 分层画布 run/operation/event 投影（additive 建表，不推动 `SCHEMA_VERSION`；`schema_watermark()` 仅供断言，版本以 [schema.rs](../../crates/store/src/turso_store/bootstrap/schema.rs) 为准）；当前 activation 的 Error 仅在 paused/blocked 且阶段不变、无取消请求、来源序号递增和身份不变时可恢复为 Done，generation CAS 与暂停状态保留。
- [project.rs](../../crates/store/src/project.rs)、[project_links.rs](../../crates/store/src/turso_store/projects/project_links.rs) — TODO 看板及执行引用；关联只保存能力 ID、执行 ID、类型、名称和创建时间，删除关联不删除节点执行；排序按专项或未归属范围校验，拒绝缺失或外部 TODO，在 Turso 事务内提交。
- [project/summary.rs](../../crates/store/src/project/summary.rs)、[turso_store/projects/project.rs](../../crates/store/src/turso_store/projects/project.rs) — schema v37 在现有项目与专项表保存 `current_summary`、`summary_updated_at`；显式正文补丁在对象事务中记录时间，未传正文保持原概述时间，概览随记录序列化。[summaries.rs](../../crates/store/src/turso_store/projects/project/summaries.rs) 用一张共享历史表保存每个对象最近 10 个版本，初始化现有内容；新增版本、回滚和超额清理与正文共享事务，删除对象清理自身历史。
- [project/acceptance.rs](../../crates/store/src/project/acceptance.rs)、[turso_store/projects/project/acceptance.rs](../../crates/store/src/turso_store/projects/project/acceptance.rs) — 验收前提的纯比较及事务核验；`patch_todo_accepted` 在同一写事务检查任务要求、归属、最新执行和依赖，再提交看板补丁；过期验收返回 `AcceptanceConflict`。
- [turso_store/projects/project/order.rs](../../crates/store/src/turso_store/projects/project/order.rs) — 项目内专项排序复用 `sort_key`；事务内核对项目、完整原顺序与成员集合后统一写入，冲突或写入失败不留部分修改。读取按 `sort_key`、创建时间、ID 排序，不触及概述历史。
- [project/tags.rs](../../crates/store/src/project/tags.rs)、[turso_store/projects/project/tags.rs](../../crates/store/src/turso_store/projects/project/tags.rs) — Tag 范围解析与关联整理；全局定义使用空 `scope_id`，项目或专项目标必须存在。同名定义按专项、项目、全局优先；名称与归属可修改并保留 ID，按保存后的名称重匹配已有选择，不给未选 TODO 添加关联，Tag 与 TODO 变更原子提交。
- [project/people.rs](../../crates/store/src/project/people.rs)、[turso_store/projects/project/people.rs](../../crates/store/src/turso_store/projects/project/people.rs)、[schema/project_people.rs](../../crates/store/src/turso_store/bootstrap/schema/project_people.rs) — schema v36 用一张关系表保存三层独立的负责人和关注人；用户外键与对象删除触发器清理关系，读取批量组装人员字段，写入在调用方事务内验证用户并替换指定角色。迁移不修改凭据。
- [project_runs/todos.rs](../../crates/store/src/turso_store/projects/project_runs/todos.rs)、[todo_patch.rs](../../crates/store/src/turso_store/projects/project_runs/todo_patch.rs) — TODO 数据读写和字段补丁；人员与 Tag 修改共用对象事务，未传人员字段保持原值，负责人显式 `null` 与关注人空数组分别清空。
- [schema/catalog.rs](../../crates/store/src/turso_store/bootstrap/schema/catalog.rs) — schema v32 的专项、Tag 与 TODO 关联结构；迁移校验专项复制结果，解除旧里程碑的 TODO 归属并移除旧容器，保留执行记录。
- [turso_store/bootstrap/schema/project_links.rs](../../crates/store/src/turso_store/bootstrap/schema/project_links.rs) — schema v33 在事务内复制、校验并替换 TODO 执行引用表，移除结论与同步状态缓存，保留执行关联和手工看板字段。
- [turso_store/bootstrap/schema.rs](../../crates/store/src/turso_store/bootstrap/schema.rs) — 初始化先读取版本，超出当前支持版本时在表结构 DDL 前拒绝打开；已有旧版本记录或版本跟踪前的会话表时创建历史项目结构，再运行升级链，支持只有部分应用表的旧数据库
- `src/store/contract/` 与 `src/turso_store/impl_methods/` 分组组装 Store 接口和实现；`schema/migrations.rs` 保存共享表升级链。旧大脑专属表不再创建，历史数据由经过核准的维护清单单独清理。

- [fleet/report/rows.rs](../../crates/store/src/fleet/report/rows.rs)：批量核验索引不可变字段，仅写入新增或变化行，冲突整批回滚。
- [fleet/handoff/pending.rs](../../crates/store/src/fleet/handoff/pending.rs)：先筛选 prepared 回执，再读取冻结请求，校验 ID 与类型一致性。
- [fleet/handoff/capacity.rs](../../crates/store/src/fleet/handoff/capacity.rs)：复用活跃票据部分索引，保持全机 FIFO 与事务内容量复核；容量恢复按 ticket、execution、runtime 三重身份校验，只恢复精确匹配的 running 票据，重复恢复幂等。
- [project_store/reopen.rs](../../crates/store/tests/project_store/reopen.rs)：关闭重开后核验项目、Tag、看板、执行引用与旧运行结果；无效 Tag 修改不能部分提交。完整存储回归入口为 [project-store-tests.yml](../../.github/workflows/project-store-tests.yml)。

- `fleet/handoff/receipts.rs` 的 `execution_names` 批量投影冻结 Assignment 中的目标、定义名称和 Brain 计划标题；名称不写回执行索引或已有运行数据。

能力详情的 `content_version` 指纹包含记录和有序样例。带版本的更新在 `BEGIN IMMEDIATE` 内重读比较，再原子提交文字、样例和向量；冲突抛出 `BrainContentConflict`，不写任何一部分。见 [brain 持久化](../../crates/store/src/turso_store/execution/brain.rs)。

[fleet/catalog.rs](../../crates/store/src/fleet/catalog.rs) 为目录删除提供只读候选：全部定义、不可变计划版本、非终态及缺少索引的 Assignment、未完成派发回执。SQL 先按名称筛选，Control 再按完整 JSON 校验引用；不删除执行历史。

## 连接、事务与维护

- 每个文件在接收操作前准备 4 个读连接和 1 个写连接，同一进程按规范路径复用连接池；文件库启用多进程 WAL，持久化使用 FULL。数据库版本仍为 39。新连接初始化与活动事务互斥，避免 WAL 初始化整理干扰已有读锁。
- 读、写各有独立执行线程；入口按请求顺序限制在途操作，写事务逐个完成。一个完整操作始终使用同一连接，多次读取共享快照；已经入队执行的写入不因调用者取消而丢弃。失败或未完成事务回滚，未能清理的连接不回池。
- 会话清单沿活跃时间索引定位当前页，再查询当前页预览，短 JSON 直接读取，长内容和含 NUL 的历史内容保留原截取规则；批量消息和事件复用 SQL，事务开始与提交语句同样缓存；单条事件使用 INSERT 自身的原子提交，批量事件继续共享显式事务。新增序号直接读取本连接最后插入的主键。
- `.db.engine.json` 标识文件所属引擎。在线查询、事务和备份通过 `opencoder-cli storage` 使用 Turso；Python SQLite 仅处理未切换的文件、已停止的库或离线快照，不能与在线 Turso 混开。
- [并发与取消](../../crates/database/tests/concurrency.rs)、[多进程与备份](../../crates/database/tests/processes.rs)、[混合负载基准](../../crates/store/examples/storage_mixed.rs) 覆盖隔离、写入收尾和性能门槛。
