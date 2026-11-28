# 数据库多写者 WAL：取消 `<db>.writer.lock` 单写者所有权，改为共享/独占维护租约

一句话：turso 升到 `0.9.0-pre.2` 后，文件库的每一次可写打开都加入引擎的**多进程共享 WAL**
（`<db>-wal` + 协调文件 `<db>-tshm`），任意数量的进程（TUI、`run`、web、CLI 命令）可以同时以写者身份
打开同一个库，事务由引擎的 WAL 写锁 + `BEGIN IMMEDIATE` 有界 Busy 重试串行化；取代"第二个写者
`AlreadyLocked` 快速失败"的是 **维护租约**（`<db>.maintenance.lock`，`flock`）：写者池整个生命周期持
**共享**租约，离线维护（TRUNCATE checkpoint、可写 bridge）取**独占**租约，写者仍存活时快速失败
"writers still active"。显式 `Intent::{Writer, Reader}` 打开意图与只读子命令保持不变。

## 背景与决策

- 单写者 `flock`（`writer_lock.rs`）把"正在运行的 TUI"变成了数据库的拥有者：同一数据目录再开
  `run`/web/第二个 TUI 会在打开阶段失败，`todos interrupt` 只能靠 intent 文件排队等待 driver 应用，
  `ts -c`/legacy 迁移遇到被持有的 store 只能推迟。这些都是锁模型的代价，不是产品语义。
- turso `0.9.0-pre.2` 的 `experimental_multiprocess_wal` 已经提供跨进程的 WAL 协调文件
  `<db>-tshm`（替代旧版的 `-shm`）与 WAL 写锁；`turso_core` 的 `PRAGMA wal_checkpoint` 也按
  checkpointer 锁仲裁多进程周期 checkpoint。单写者锁不再需要。
- 仍需要回答的唯一问题是"谁在碰这个文件"：离线 TRUNCATE 会原地重写 WAL，不能在有写者时进行；
  可写 bridge 是离线维护，不能和写者并存。答案是一个租约文件而不是写者锁：**共享租约 = 写者存活，
  独占租约 = 离线维护进行中**，两类请求互斥，由 `flock` 的共享/独占语义直接表达。

## 模型

### 写者

- `Database::open(path)`（默认 `Intent::Writer`）：取 `<db>.maintenance.lock` 的**共享**租约并持有
  到池释放（`pool.rs` `Inner.maintenance`，字段声明在最后，引擎与全部连接释放后才释放租约），然后
  以多进程 WAL 打开引擎。`TursoStore::open` / `TsRegistry::open`（store 侧）都是这条路径。
- 事务：`database::busy::run_tx(&connection, "BEGIN IMMEDIATE", ..)`，退避 25ms→1s、预算 120s、
  按事务计；引擎的 WAL 写锁保证同一时刻只有一个写事务。单语句自动提交的写入**没有**重试，
  所有写路径都必须在 `run_tx` 内。
- 周期 checkpoint：每个写者池注册一个常驻句柄（一条专用连接），每个进程各跑自己的 PASSIVE
  （60s / 1 MiB 门限），引擎的 checkpointer 锁仲裁，busy 即跳过；池释放即 disarm，句柄随之下一轮
  剔除。`run_once` 复用存活池的句柄，否则在自己的共享租约下开私有引擎——另一个写者存活从来不是
  拒绝的理由。

### 首次打开（[engine.rs](../../../crates/database/src/engine.rs)）

- `<db>-tshm` 不存在即"首次打开"：全新文件，或从没有协调文件的旧引擎（0.8.3）升级、或只拷回
  裸 `.db` 文件。引擎先建空文件再写头部，中间被另一个进程打开会看到短文件（`Error::Corrupt`
  "shared WAL coordination file is smaller than the coordination header"）。
- 协议：首个写者先取**独占**租约再打开引擎（初始化期间没有别的写者能加入），完成后降级为共享
  租约；同一时刻的其他打开者对共享租约的请求会被独占持有者拒绝（`MaintenanceActive`），
  `acquire_shared_waiting` 在 120s 预算内等待后重试。协调文件仍在初始化的打开在 10s 预算内重试。
- 引擎自身**从不删除** `-wal`/`-tshm`（TRUNCATE 后 WAL 留作 0 字节文件）。它们缺失只会发生在上面
  三种首次打开场景；不要在有进程打开着库时手工删除伴随文件——存活进程仍映射着旧 inode，之后
  重新打开会复用进程内的旧引擎而看不到新 WAL。

### 离线维护

- TRUNCATE（`maintenance::checkpoint`）取**独占**租约并持有到文件自包含：任一写者池存活即快速失败
  `WritersActive`（"writers still active"）；reader 不取租约，仍持有 WAL 快照的 reader 让引擎报 busy
  而不是"写者存活"。非冲突失败（权限、I/O）保留原错误。
- 可写 bridge（`storage serve --writable`）同样取独占租约到 stdin 关闭；只读 bridge 不取租约，
  写者运行时照常可巡检。
- 租约 API（[maintenance_lock.rs](../../../crates/database/src/maintenance_lock.rs)）：
  `acquire_shared`（被独占持有者拒绝 → `MaintenanceActive`）、`try_acquire_exclusive`
  （被写者拒绝 → `WritersActive`）、`is_writers_active` / `is_maintenance_active`、
  `MaintenanceLease::{database, lock_path, mode}`。

### Reader（不变）

`Database::open_reader` / `Intent::Reader`、`TursoStore::open_reader`、`TsRegistry::open_reader`：
不取租约、引擎只读、连接 `query_only`、不建 marker、不建库、不迁移、不 checkpoint，`write` 被拒；
存活写者已提交的 WAL 帧经共享 WAL 可见。

## 调用点语义

- 写者：TUI / web / node / run 与全部变更命令（`session delete/import`、`todos` 的
  run / resume / interrupt、`ts` 的 start / resume 注册、cleanup、delete、legacy 迁移）。再开一个写者
  不再失败；它只是在事务层排队。
- `todos interrupt`：直接以写者打开并提交挂起（CAS 提交 `Suspended` + 一条带 `reason` 的
  `workflow_interrupted` 事件），不等 driver 确认；driver 在下一步边界看到已持久化的挂起。
  CLI 不再写 intent 文件，但会删除旧构建留下的 intent，避免之后的 run 继承旧挂起。
  `crates/todos` 的 `interrupt_intent`（runner 消费侧）保留。
- ts registry（`ts.db`）：TUI 镜像整个生命周期只持 reader，每次镜像变更 `TsRegistry::open` 一个
  用完即弃的写者；ts CLI 只在注册 / 删行期间持写者，attach / spawn 期间不持有；`ts -l` 与
  attach-only resume 只读。legacy 扫描 reader 优先，schema 过旧时退回迁移写者（不再有
  `MigrationOutcome::Deferred`）。`ts -c` / `ts -d` 直接写进存活会话正打开着的 store
  （不再有 `kept_in_use` / `DELETE_STORE_RETRY`）。
- Reader：`session list/show/export`、`todos list/show/events`、`ts -l`、`storage serve`（默认）/
  `query` / `backup`。库不存在时 `list` 报告空、`show` / `export` / `events` 报告不存在，都不为了
  报告而建库。`session export` 携带 orphan 输入且不修复。
- 旧 schema：纯读打开不会迁移，报告记录的 schema 版本与补救办法，引擎自身的错误保留为 cause
  （每条读自己的语句、自己的错误）。

## 变更

- Stage 1：`Cargo.toml` / `Cargo.lock` turso 系列 `=0.9.0-pre.2`；
  `crates/database/src/lib.rs` `STORAGE_ENGINE_VERSION = "0.9.0-pre.2"`。
- `crates/database`：`maintenance_lock.rs`（新）、`engine.rs`（新：`coordination_path`、
  `is_coordination_file_uninitialized`、`open`、`acquire_shared_waiting`、`open_writer`）、
  `pool.rs`（经 `engine` 打开、`Inner.maintenance`）、`checkpoint/{mod,handle,periodic}.rs`
  （句柄绑定弱引用、无 owner_token）、`maintenance.rs` 与 `maintenance/bridge.rs`（独占租约）；
  删除 `writer_lock.rs` 与 `AlreadyLocked`。
- `crates/store`：`ts_registry.rs` 重写（无 `open_writer` / `WriterRetry` / `writer_held` /
  `is_already_locked`）；`TursoStore::open_reader` 保留。
- `crates/local`：`ts/{actions,registry}.rs` 回到直接 `TursoStore::open` / `TsRegistry::open`，
  `ts/lock_tests.rs` → `ts/concurrency_tests.rs`；`todos_interrupt.rs` 只剩直接写路径。
- `crates/tui`：`ts_mirror.rs` 每次 `mutate` 一个 `TsRegistry::open`，构造函数不再带重试策略。
- `crates/ctl`：`storage serve --writable` 文案改为独占租约语义。

## 测试覆盖

- `crates/database/tests`：`maintenance_lock.rs`（子进程写者：共享/独占互斥、`WritersActive`、
  `MaintenanceActive`）、`first_open_race.rs`（5 个子进程同时对缺失文件 / 无伴随文件的库首次打开，
  全部成功且每行恰提交一次）、`processes.rs`（多子进程写者 + reader）、`busy_contention.rs`、
  `concurrency.rs`、`checkpoint.rs` / `checkpoint_binding.rs`（多池句柄绑定）、`maintenance.rs`
  （TRUNCATE 等待池释放、reader 快照让 TRUNCATE busy 但不算写者）、`database_paths.rs`、
  `pool_growth.rs`、`multi_writer_soak.rs`（`#[ignore]` 压测：4 个写者子进程，杀掉其中一个后其余继续
  推进，随后独占租约可得、TRUNCATE 后 `wal_size == 0`、前缀和与 `integrity_check` 通过）。
- `crates/store/tests`：`reader_open.rs`、`schema_bootstrap_race.rs`（4 个子进程同时对全新文件
  `TursoStore::open` + `TsRegistry::open`）。
- `crates/local`：`ts/concurrency_tests.rs`（子进程持有者：legacy 导入、cleanup/delete、registry 写
  并存）、`todos_interrupt/tests.rs`、`tests/*_reader_dispatch.rs`、`tests/*_reader_old_schema.rs`、
  根目录 `tests/todos_interrupt_e2e.rs`（真实二进制：直接挂起、无 intent 文件、恰一条事件）。
- `crates/tui`：`ts_mirror_tests.rs::mirror_mutations_commit_beside_a_live_writer`。

## 验证

- `cargo test -p opencoder-database -p opencoder-store -p opencoder-session -p opencoder-local
  -p opencoder-tui -p opencoder-control` 与 `cargo test --test todos_interrupt_e2e` 全绿；
  `cargo clippy --all-targets` 与 `cargo fmt --check` 零告警。
- `first_open_race` ×10、`schema_bootstrap_race` ×8 连续通过（修复前后者约 1/3 失败）。
- 真实库副本（含旧伴随文件）以 0.9.0-pre.2 打开 / 查询 / TRUNCATE / 重开查询通过。
- 未运行根目录全部 e2e 套件与真实多机部署验证。

## Impact Surface

- 用户可感知：同一数据目录下多开 TUI / web / node / run 不再在打开阶段失败；`todos interrupt` 立即
  持久化挂起，不再等待 driver 确认；`ts -c` / `ts -d` 对运行中会话的 store 直接生效。
- 文件系统：文件库旁不再有 `<db>.writer.lock`；新增 `<db>.maintenance.lock`（0600，释放后保留）与
  turso 0.9 的 `<db>-tshm`（旧 `-shm` 不再使用）。
- Store：零 schema 变化。
- 运维：`storage checkpoint --writers-stopped` 与 `storage serve --writable` 要求确实没有写者存活，
  否则快速失败 "writers still active"；写者对独占维护的等待上限 120s（`MaintenanceActive`）。

## Related Docs

- [agents/foundation/store/index.md](../../../agents/foundation/store/index.md)
- [agents/frontend/local/index.md](../../../agents/frontend/local/index.md)
- [agents/frontend/tui/index.md](../../../agents/frontend/tui/index.md)
- [agents/platform/ctl/index.md](../../../agents/platform/ctl/index.md)
- [repo-memory.md](../../../repo-memory.md)
