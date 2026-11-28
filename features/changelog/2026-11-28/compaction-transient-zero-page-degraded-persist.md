# compaction 持久化遇到瞬时零页不再整段失败 + checkpoint 常驻专用连接

一句话：`compaction failed: persist compaction metadata: build session activity index: Invalid page type: 0`
不是单点损坏，而是「turso 0.8.3 语句回滚只回滚一半 + 周期 checkpoint 每 60 秒 `connect()` 触发
shared-WAL authority 修复 + compaction 持久化没有降级/退避」三层叠加。本次只动应用层可安全修复的部分：
① `sessions::update` 在活跃索引重建失败时降级为不刷活跃时间戳（与消息追加一致）；② 索引重建
对瞬时不可读页做有界退避重试；③ compaction 元数据写失败只退避重试持久化、不重跑 summarize，
并用标记错误让 runner 停止无退避重试；④ 周期 checkpoint 改为常驻专用连接：建池阶段一次
`connect()`，之后每轮复用（仅 pass 失败时丢弃并在下一轮重开），不再每 60 秒 `connect()`。

## 背景与决策

- 错误链逐段核对：`build session activity index` 只出现在 `is_missing_index_entry` 命中后的修复
  路径，说明每次失败是「`UPDATE sessions` 先报 `IdxDelete` 失步 → `DROP INDEX` 重建 → 全表扫
  读到全 0 页」的双重损坏信号。全 0 页不可能来自短读（那会报 `ShortRead`），只可能来自页缓存/
  WAL frame 索引/回滚残留；偶发成功说明是瞬时内存视图错误。
- 引擎侧（本次不改）：turso 0.8.3 `rollback_to_snapshot` 先 `clear_dirty`/`try_unpin`，再
  `cache.truncate`，失败会跳过 `wal.rollback` 与游标失效；0.9.0-pre.2 已把
  `invalidate_all_cursors()` 前置，并修复 shared-WAL 重开时清掉同进程兄弟连接 reader slot 的问题。
- 触发器核对（turso_core 0.8.3 源码）：旧 `run_once` 每轮的 `Builder::build()` 在池存活时命中进程内
  `(dev, ino)` 注册表、直接复用同一核心 `Database`（`database.rs:1289-1304`），并不重开文件；真正每轮
  执行的是 `connect()` → `init_pager` → `ShmWalCoordination::new` → `seed_or_sync_authority()`
  （`wal.rs:1511`）。当库经磁盘扫描路径打开（已有库 + 非空 WAL 被首个进程打开，或 `nbackfills != 0`
  的 Exclusive 重开）时 `loaded_from_disk_scan` 为 true 且不复位，于是每次 connect 进入
  `repair_or_reseed_authority_from_local_disk_scan`（`wal.rs:1821-1824`）：无条件
  `repair_transient_state_for_exclusive_open`（Linux OFD 对同 fd 的 reader slot 探测可清同进程兄弟槽），
  并可能瞬时清空 durable frame index——与「全 0 页 = 瞬时内存视图错误」形态一致（运行时未复现）。
  因此「复用 `turso::Database` 但每轮仍 `connect()`」不消除触发器，必须让连接常驻。
- 应用层放大器：`sessions::update` 用无降级分支的 `repair_and_retry`，而消息追加用
  `repair_and_retry_or`（降级 = `touch=false`）；compaction 重试 3 次无退避且每次重跑 summarize。
- 决策：
  - `sessions::update` 抽出 `patch_plan(patch, id, touch)`，`updated_at` 仅在 `touch=true` 进 SET；
    改走 `repair_and_retry_or`，降级分支只写非索引列（summary/summary_seq 等）。
  - `rebuild_activity_index` 的探测 `CREATE INDEX ..._rebuild` 与 swap 事务（`swap_activity_index`，
    其 `CREATE INDEX` 也是全表扫，事务体幂等）都包 `retry_on_unreadable`，仅 `is_unreadable_storage`
    （`SQLITE_CORRUPT` 或 worker panic）命中才重试，退避 250ms/1s，共 3 次；其他错误原样上抛。
  - compaction：`persist_summary` 对 `update_session` 的所有写失败做有界退避重试（250ms/1s，
    不可读页是最常见形态，但不按错误类型区分），仍失败则包成 `PersistFailure` 标记；runner
    识别标记后直接结束（不重跑 summarize），并发出 `compaction summary not persisted yet;
    retrying n/2` 状态事件（计重试次数）；`PersistFailure` 的 Display 不再重复内嵌 source 文案。
  - checkpoint：`CheckpointHandle` 持有池的 `turso::Database` 与一条常驻专用连接；新建池时在建池
    阶段（尚无在途操作）`prime()` 一次 `connect()`（prime 失败只告警、不阻断建池，首轮 pass 内补一次），
    之后每轮 `checkpoint_passive` 复用该连接（成功路径 `connection_opens` 恒为 1，可观测），连接锁
    顺带串行化并发 pass；pass 失败则与池「失败连接不回池」的策略一致立即丢弃该连接（pager 可能保留
    失败时的页视图），下一轮 pass 重开一条（`connection_opens` +1），busy 结果不算失败；池注册命中时
    保留已 prime 的句柄（`register_if_absent`）；`handle_for` 仅在无存活池句柄时才开独立句柄
    （`standalone_opens`；并行多写者 WAL 工作后，池已释放的失效句柄会先被剔除再开独立句柄，见
    [多写者 WAL changelog](database-multi-writer-wal-and-maintenance-lease.md)）。
  - `is_unreadable_storage` 从 `opencoder-store` 导出，供 session 层分类。

## 未做（明确留待后续）

- P1-2 `maintenance.rs`/`maintenance/bridge.rs` 的短生命周期句柄：`maintenance.rs` 全部
  `.read_only(true)`，不走 exclusive 修复；`bridge` 是显式维护客户端，按连接生命周期持句柄。暂不改。
- P1-3 单写者进程 advisory lock：会改变 TUI/web/CLI 并存行为，需产品侧确认（后续由
  [多写者 WAL + 维护租约](database-multi-writer-wal-and-maintenance-lease.md) 取代：不设单写者，
  仅离线维护独占）。
- `retry_on_unreadable` 的分类器仍用 `is_unreadable_storage`（任意 `Corrupt` 或 worker panic）：真实
  坏行会让每次 `sessions::update` 多做 3 次全表建索引 + 1.25s，且全程持有写者 permit。可收窄为
  `Invalid page type` + `JoinError`（后者是兄弟连接 reader slot 被清后的 panic 形态），需单独决策。
- Linux OFD 同 fd 探测不冲突属 `fcntl(2)` 语义，未在仓内运行时验证；连接常驻后每轮不再 `connect()`，
  触发器应随之消失，以观察期为准。
- checkpoint 连接的惰性重开（prime 失败后的首轮，或 pass 失败丢弃后的下一轮）直接走句柄自己的
  `connect()`，不经池 `connect()` 的在途事务互斥；它只跟随失败而非定时发生，若观察期出现同进程
  reader 被扰动的迹象，再让重开回到池的静默时刻。
- checkpoint 注册表原为只增不删（每路径 1 个引擎随进程存活）；并行多写者 WAL 工作后，池已释放的失效句柄由
  周期 pass / `handle_for` 剔除，池仍存活的句柄随进程存活（见
  [多写者 WAL changelog](database-multi-writer-wal-and-maintenance-lease.md)）；`STANDALONE_OPENS` 为进程级计数，跨并行
  测试断言不变较脆弱。
- session 集成测试直接构造 `turso::Error`（dev-dep），与「引擎细节不出 store」的约定相悖，可由 store
  提供测试构造器后移除。
- P2 升级 turso 0.9.0-pre.2：pre-release 风险高，需独立 commit、定向回归与回滚预案；本次先移除
  应用层触发器并观察。（后续已完成，见
  [多写者 WAL changelog](database-multi-writer-wal-and-maintenance-lease.md)。）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `patch_plan` 仅在 touch 时写 `updated_at`，绑定值顺序逐一锁定；无活跃戳时两种计划一致；仅活跃戳的降级计划为空；空 patch 不生成语句 | `patch_plan_touches_the_activity_stamp_only_when_requested`、`patch_plan_without_an_activity_stamp_is_touch_independent`、`activity_only_patch_has_no_degraded_statement`、`empty_patch_writes_nothing` | `crates/store/src/turso_store/conversation/sessions/parts/part_2.rs` |
| 重建（探测与 swap 共用的 `retry_on_unreadable`）遇瞬时不可读页：前两次失败后成功 / 非不可读错误立即上抛 / 预算耗尽后上抛 | `retry_on_unreadable_recovers_from_a_transient_page`、`retry_on_unreadable_surfaces_other_errors_immediately`、`retry_on_unreadable_gives_up_after_the_budget` | `crates/store/src/turso_store/conversation/sessions/activity.rs` |
| 真实形状（索引失步 + 另一会话行 overflow 链截断）：`update_session` 仍落 summary/summary_seq、不刷 `updated_at`、列表索引保留 | `update_session_lands_without_activity_touch_when_the_index_cannot_be_rebuilt` | `crates/store/tests/session_activity_index_repair.rs` |
| 既有真实形状回归：追加降级落地、索引自愈、v40 迁移重建 | `append_lands_without_activity_touch_when_the_index_cannot_be_rebuilt`、`append_self_heals_a_desynced_activity_index`、`migration_v40_rebuilds_expression_index_and_backfills_activity` | `crates/store/tests/session_activity_index_repair.rs` |
| `PersistFailure` 穿过 anyhow context 仍可识别；普通错误不误判 | `persist_failure_is_detected_through_anyhow_context` | `crates/session/src/compaction/tests.rs` |
| compaction 写失败一次后成功：summarize 只跑 1 次、写共 2 次、发出 `retrying 1/2` 状态 | `compact_retries_the_write_without_rerunning_the_summarizer` | `crates/session/tests/compaction_error_propagation.rs` |
| 持续失败：最外层消息恰为 `persist compaction metadata`（store 错误留在链中）、带 `PersistFailure` 标记、写共 3 次、summarize 1 次、发出 `retrying 2/2` | `compact_returns_err_when_store_rejects_metadata_persistence` | `crates/session/tests/compaction_error_propagation.rs` |
| 非不可读的普通写失败同样重试到预算并打 `PersistFailure` 标记 | `compact_retries_every_write_failure_not_only_unreadable_storage` | `crates/session/tests/compaction_error_propagation.rs` |
| checkpoint 复用池句柄：同路径同 Arc、`standalone_opens` 不增长、gated pass 不重开 | `periodic_checkpoint_reuses_the_pool_handle` | `crates/database/tests/checkpoint.rs` |
| checkpoint 常驻专用连接：建池 prime 后 `connection_opens` 恒为 1，多轮成功 pass 不再 `connect()` | `periodic_checkpoint_keeps_one_connection_per_handle` | `crates/database/tests/checkpoint.rs` |
| checkpoint pass 失败即丢弃专用连接：失败后槽位为空、下一轮重开一次（`connection_opens` 1→2）、之后不再增长 | `failed_pass_discards_the_dedicated_connection` | `crates/database/src/checkpoint.rs` |
| swap 事务幂等：从「探测已建、列表索引已 DROP」的半完成态出发，swap 收敛到只剩 `idx_sessions_activity`，重复执行不变 | `swap_converges_from_a_half_done_swap` | `crates/store/src/turso_store/conversation/sessions/activity.rs` |
| checkpoint 既有门限/合并/忙时不阻塞回归 | `run_once_skips_below_threshold`、`run_once_merges_wal_above_threshold`、`run_once_reports_busy_without_blocking`、`wal_path_matches_turso_layout` | `crates/database/tests/checkpoint.rs` |

- 定向回归（评审收尾 + 评审遗留项处理后重跑）：
  - `cargo test -p opencoder-store --test session_activity_index_repair` → 4 passed / 0 failed
  - `cargo test -p opencoder-store --lib -- sessions:: store::errors` → 19 passed / 0 failed（新增 swap 幂等 1 个）
  - `cargo test -p opencoder-session --test compaction_error_propagation` → 3 passed / 0 failed
  - `cargo test -p opencoder-session --lib -- compaction:: runner::` → 85 passed / 0 failed
  - `cargo test -p opencoder-database --test checkpoint` → 6 passed / 0 failed
  - `cargo test -p opencoder-database --lib` → 10 passed / 0 failed（含并行单写者锁工作新增的 1 个用例、
    本次新增的丢弃连接用例 1 个）
- Clippy：`cargo clippy --no-deps -p opencoder-store -p opencoder-session --all-targets -- -D warnings`
  → 零警告；database 的 clippy 当前被并行进行中的单写者锁改动挡住（`pool.rs:88` 静态表类型
  `type_complexity`，非本次改动引入）；以 `-A clippy::type_complexity` 对 `--lib --test checkpoint`
  复核为零警告，即本次改动文件无告警。
- 格式：本次改动文件 `rustfmt` 无差异；`cargo fmt --all --check` 的差异全部位于并行新增的
  `tests/processes.rs`、`tests/writer_lock.rs`（后者已随单写者锁一并移除）。
- 测试范围：本次修改及其直接影响的 store/session/database 链路；未运行全量测试。
