# 存储引擎迁移到 rdb + 事件驱动调度基建

一句话：`Store` / `ProjectStore` / `FleetStore` / `TsRegistry` 的生产实现
从 turso 切到 RocksDB（`rdb`）主后端，并新增 `opencoder-event` 作为事件
驱动调度的通用基建（当前未接入任何生产调度器）；全局暂停只拦截分发、
不拦截事件日志追加，Brain 的层屏障/暂停/恢复/终态信号在同一原子批内
镜像到平台日志。

## 行为口径

- **单拥有者（进程内）**：`rdb_store::engine::RdbDb` 按目录注册 `Weak`
  核心，同进程重复打开共享同一 RocksDB 句柄与写租约；每次修改走一个
  `WriteBatch` 原子提交，失败不留下半批数据。
- **单拥有者（跨进程）**：打开目录时对同级 `<db-dir>.owner.lock` 取
  fs2 exclusive lock；被他进程持有时返回 busy error，错误信息包含数据库
  路径、lock 路径与持有者 pid/进程描述等非敏感元数据。不支持多进程并发
  写同一目录。
- **路径策略**：会话库 `opencoder.db.rdb`、运行时库 `runtime.db.rdb`
  优先；仅当旧 stem（`opencoder.db` / `runtime.db`）已是目录且 `.rdb`
  目录尚不存在时回退旧开发目录布局；旧普通 SQLite 文件绝不作为 RocksDB
  目录打开。生产入口统一走 `opencoder_core::session_store_rdb_dir` /
  `opencoder_core::runtime_store_rdb_dir`（`crates/core/src/data_dir.rs`）。
- **KV 数据模型**：根行、子行与二级索引各占一个 key；父行删除用
  `delete_range` 代替 SQL 级联。原有 DTO/公共 API 不变。
- **平台事件日志**：`evt/<seq:020>` 追加式全局日志；监听器 `evl/<id>`，
  分发等待 `evw/<execution>`，全局门 `gate/scheduler`。
- **暂停/恢复/追加（基建口径）**：`SchedulerGate` 只影响 `dispatch_gated`；
  暂停期间 `EventLedger::append` 继续工作。`TriggerAction::ResumeExecution`
  携带 `append` 输入，恢复后按日志游标重放。
- **event 生产边界（如实声明）**：`SchedulerGate` 没有生产 setter，默认
  放行；`run_pump` / `dispatch_gated` / `TriggerAction`（含
  `ResumeExecution`）当前没有任何生产 `ActionExecutor`、outbox 或控制面
  接线。平台事件的唯一生产 producer 是 `commit_brain_layered` 的 Brain
  信号 additive mirror。`execution.paused` 与 `brain.operation_terminal`
  是保留词汇，当前无 producer。未来接线时 `ResumeExecution.append` 必须
  继续走 worker v4 累计 human-input budget，不允许事件重放绕过准入。
- **Brain 接入**：`commit_brain_layered` 将 `layer_barrier_reached`、
  `operation_terminal`、`run_paused`、`run_resumed` 映射为平台事件，与
  Brain 投影、outbox 同批提交；Brain 自身语义与 generation 围栏不变。
  run 级 pause/resume 不触碰全局 scheduler gate；`operation_terminal`
  事件缺 `execution_id` 时整笔提交被拒绝。
- **BuildInfo**：`crates/core/src/version.rs` 的
  `BuildInfo.storage_engine` 由 `"turso"` 改为 `"rdb"`；该字段只描述主
  会话/运行时存储引擎，Ontology 独立后端不受此字段描述。
- **后端范围**：Ontology 仍使用独立 SQLite（`opencoder-database`）；
  `opencoder-store` 删除 turso 实现与依赖。
- **关联变更**：旧 SQLite/turso 会话库显式迁移见
  [legacy-session-store-migration.md](legacy-session-store-migration.md)；
  向量载荷加固见
  [rdb-vector-payload-hardening.md](rdb-vector-payload-hardening.md)
  （细节不在本文重复）。

## 变更清单

- `crates/event/` — 事件词汇、监听器注册/匹配、广播总线、触发执行、
  `EventLedger`、`SchedulerGate` 与 `dispatch_gated`。
- `crates/core/src/data_dir.rs` — `.rdb` 目录解析策略
  （`session_store_rdb_dir` / `runtime_store_rdb_dir`）。
- `crates/core/src/version.rs` — `BuildInfo.storage_engine` = `"rdb"`。
- `crates/store/src/rdb_store/` — 会话/消息/事件/输入、TODO、执行账本、
  项目目录、fleet、向量与 TS 注册表的 rdb 实现。
- `crates/store/src/rdb_store/engine.rs` — 单句柄/写租约 + 跨进程
  owner lock（`<db-dir>.owner.lock`）。
- `crates/store/src/store/contract/part_4.rs` — 平台事件/监听器/等待/gate
  的 `Store` 契约（默认拒绝，rdb 覆盖）。
- `crates/store/src/event_ledger.rs` — `Store` → `EventLedger` 适配器。
- `crates/store/src/rdb_store/events.rs` — 事件日志、监听器、等待与 gate
  的原子持久化。
- `crates/store/src/rdb_store/execution/brain_layered.rs` — Brain 信号镜像
  （`run_paused` / `run_resumed` 与 `operation_terminal` execution-id
  guard）。
- `crates/store/src/turso_store/` — 删除。
- `agents/foundation/store/index.md`、`agents/foundation/event/index.md`、
  `repo-memory.md` — 索引同步。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 事件日志追加与游标 | `append_assigns_monotonic_seq_and_reads_after_cursor` | `crates/store/src/rdb_store/events.rs` |
| 监听器 upsert/删除 | `listener_upsert_delete_and_order` | `crates/store/src/rdb_store/events.rs` |
| 等待模式匹配与清除 | `waits_match_glob_and_clear` | `crates/store/src/rdb_store/events.rs` |
| gate 持久化 | `gate_roundtrip` | `crates/store/src/rdb_store/events.rs` |
| `Store` → `EventLedger` 适配 | `adapter_delegates_log_listener_and_wait` | `crates/store/src/event_ledger.rs` |
| 暂停时仍镜像层屏障 | `layer_barrier_is_mirrored_to_platform_log_while_paused` | `crates/store/tests/brain_layered_v4.rs` |
| 暂停/恢复/终态信号镜像 | `brain_control_signals_mirror_into_platform_log` | `crates/store/tests/brain_layered_v4.rs` |
| terminal 缺 execution_id 整笔拒绝 | `operation_terminal_requires_an_execution_identity_for_platform_mirror` | `crates/store/tests/brain_layered_v4.rs` |
| 暂停门跳过分发 | `paused_gate_skips_dispatch_without_touching_listeners` | `crates/event/src/executor.rs` |
| 暂停/恢复转换 | `pause_keeps_existing_reason_and_resume_is_idempotent` | `crates/event/src/domain/gate.rs` |
| owner lock：同进程共享核心 | `same_process_opens_share_one_core` | `crates/store/src/rdb_store/engine.rs` |
| owner lock：busy error 指名持有者 | `manually_locked_owner_file_yields_busy_error` | `crates/store/src/rdb_store/engine.rs` |
| owner lock：末句柄释放 | `owner_lock_releases_with_last_handle` | `crates/store/src/rdb_store/engine.rs` |
| owner lock：非敏感持有者元数据 | `acquire_writes_non_sensitive_owner_metadata` | `crates/store/src/rdb_store/engine.rs` |
| data_dir：`.rdb` 新布局优先 | `rdb_dir_prefers_new_rdb_name` | `crates/core/src/data_dir.rs` |
| data_dir：旧目录布局回退 | `rdb_dir_falls_back_to_legacy_directory` | `crates/core/src/data_dir.rs` |
| data_dir：绝不碰撞旧 SQLite 文件 | `rdb_dir_never_collides_with_legacy_sqlite_file` | `crates/core/src/data_dir.rs` |
| BuildInfo 存储引擎口径 | `build_info_uses_compiled_protocol_and_commit` | `crates/core/src/version.rs` |
| rdb 执行账本 | `rdb_execution` 契约套件 | `crates/store/tests/rdb_execution/` |
| rdb fleet | `rdb_fleet` 契约套件 | `crates/store/tests/rdb_fleet/` |
| rdb 报告/outbox | `rdb_ledger` 契约套件 | `crates/store/tests/rdb_ledger/` |

## 增补：分发判定显式化 + 消费者游标 + 事件泵

- **显式分发判定**：`dispatch_gated` 返回
  `DispatchDecision::{Paused, Dispatched(Vec<DispatchOutcome>)}`，暂停与
  “无监听器/全部成功”不再共用空集合语义；`dispatch`、`execute_planned`
  行为不变，单监听器失败不中断其余监听器。
- **消费者游标**：`EventLedger` 新增 `consumer_cursor` /
  `save_consumer_cursor`（i64，游标仅非负）；rdb 新增 `evc/<consumer-id>`
  KV 族，`Store` 契约新增 `platform_consumer_cursor` /
  `platform_save_consumer_cursor`（默认拒绝，避免假起点静默重放）。
  消费者 id 必须非空、有界、ascii、无空白、不含 `/`。
- **事件泵 `consumer::run_pump`**：按游标严格向后读取、暂停不前移、
  整批全部成功才落游标、失败保留游标并上报事件 seq + 失败监听器 id、
  空日志零写入。纯通用基建，未接入 Brain 调度或 outbox。

### 功能 → 测试映射（增补部分）

| 功能 | 测试 | 文件 |
|------|------|------|
| 暂停判定与空分发区分 | `paused_gate_skips_dispatch_without_touching_listeners`、`open_gate_without_matching_listeners_is_dispatched_empty` | `crates/event/src/executor.rs` |
| 消费者 id/游标校验 | `accepts_plain_ids_and_rejects_key_unsafe_ones`、`cursor_must_be_non_negative` | `crates/event/src/domain/consumer.rs` |
| 内存账本游标读写 | `consumer_cursors_are_independent_and_validated`、`events_after_reads_strictly_after_cursor` | `crates/event/src/ledger.rs` |
| 泵：暂停不前移 | `paused_gate_returns_paused_without_dispatching_or_advancing`、`paused_after_progress_keeps_saved_cursor_and_resumes_from_it` | `crates/event/tests/consumer_pump.rs` |
| 泵：成功前移一次落盘 | `successful_batch_dispatches_all_and_saves_cursor_once` | `crates/event/tests/consumer_pump.rs` |
| 泵：失败保留游标可重试 | `failing_listener_keeps_cursor_then_retry_advances` | `crates/event/tests/consumer_pump.rs` |
| 泵：空日志零写入 | `empty_log_is_idle_and_writes_nothing` | `crates/event/tests/consumer_pump.rs` |
| 泵：批边界分块 | `batch_limit_chunks_progress_then_goes_idle` | `crates/event/tests/consumer_pump.rs` |
| 泵：非法消费者 id | `invalid_consumer_id_is_rejected_before_any_work` | `crates/event/tests/consumer_pump.rs` |
| rdb 游标重开存活 | `consumer_cursor_roundtrip_survives_reopen` | `crates/store/src/rdb_store/events.rs` |
| rdb 游标输入校验 | `consumer_cursor_rejects_unsafe_ids_and_negative_seq` | `crates/store/src/rdb_store/events.rs` |
| 适配器游标委托 | `adapter_delegates_cursor_and_survives_reopen`、`adapter_validates_consumer_id_and_cursor_value` | `crates/store/src/event_ledger.rs` |

- 定向回归（撰写时结果；只覆盖本次改动及直接影响链路，非全量）：
  - `cargo test -p opencoder-core --lib version` → 12 passed / 0 failed
  - `cargo test -p opencoder-store --lib` → 49 passed / 0 failed
  - `cargo test -p opencoder-store --test rdb_execution --test rdb_ledger --test rdb_fleet` → 全部通过
  - `cargo test -p opencoder-store --test brain_layered_v4 --test store_integration` → 全部通过
  - `cargo test -p opencoder-store --test rdb_vectors` → 12 passed / 0 failed
  - `cargo test -p opencoder-event` → 全部通过
- Clippy：`cargo clippy -p opencoder-store -p opencoder-event --all-targets --no-deps -- -D warnings`
  → 零警告。
