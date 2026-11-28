# 缺陷修复 F1–F7：`ts -r` 导入后重解析、rolling storage 连接显式打开侧、活动索引 swap 失败清理、compact 状态收口、checkpoint 入口前置路径检查、写者存活判定契约

一句话：F1 `ts -r` 在注册表缺失时先导入 legacy 行、再对导入行重解析（stopped legacy 会话可按前缀命中，仍歧义的前缀照样
拒绝）；F2 rolling storage 连接显式声明打开侧（默认 `--writable`、readonly `--read-only`），release compatibility 的
`NativeSql` 默认不传 mode flag；F3 活动 swap 失败收尾同时清掉探测索引；F4 `compact` 对所有 `Err` 清 compacting 状态、
`Ok(None)` 保留 `nothing to compact yet`；F5 两个 checkpoint 入口在 acquire lease 前拒绝非 UTF-8 路径；F6
`is_writers_active` 的 error.chain 契约及其测试；F7 `session_cmd` 死分支改为显式 `unreachable!`，`ts -d` 锁冲突文案只
命名一次 store 路径。这些修复自身不新增 schema / 公开能力，Ontology 无需变化。

## 背景与决策

- 解析顺序以数据为准：导入后的行集才是前缀解析的输入，歧义检查在重解析时同样生效，不取首个匹配。
- 脚本侧显式声明打开侧：桥接默认只读后，「默认可写」的调用方须自行请求写者侧；跨版本验收脚本不给旧二进制传新 flag。
- 失败路径的收尾全部 best-effort 且幂等，不掩盖原始错误；前端状态以「最后一次 Status」为准，因此失败必须显式清空。
- lease 文件名由路径的有损渲染派生：任何会在 lease 之后才失败的输入，都在 lease 之前拒绝。

## 变更

### F1 `ts -r`：注册表缺失时先导入、再重解析（`crates/local/src/ts/resume.rs`）

- 此前 `registry == None` 时对空行集解析一次 target，legacy 导入行不参与 ID / 前缀解析，stopped legacy 会话无法按前缀命中。
- 现在：target 先对当前行集解析一次判断是否为存活 tmux 会话——存活的直接 attach，不跑导入；否则且注册表缺失时，先执行
  调用方传入的 `legacy_import`（短命写者，返回前即释放），再用同一个 `resolve_managed_id` 对导入行重解析：只有导入行
  知道的前缀得以展开，导入后仍指向多个会话的前缀被拒绝（`ambiguous global tmux session prefix … matches: …`）。
- 解析函数对输入是纯的（存活会话列表、只读行集、导入闭包），测试不依赖 tmux 与真实数据根。

### F2 rolling storage 连接显式打开侧；release compatibility 不给旧二进制传新 flag

- `scripts/platform/rolling/storage/connection.py`：`Connection` 默认显式追加 `--writable`，`readonly=True` 显式追加
  `--read-only`。桥接改为默认只读后，原先「不传 flag = 可写」的默认连接会静默变成只读；现在两侧都点名，恢复脚本
  （`maintenance/recovery/capacity.py`）的可写连接与只读连接各自落到预期一侧。
- `scripts/acceptance/release_compatibility/processes.py`：`NativeSql(binary, database, root, writable=False)`，
  默认不传任何 mode flag——旧二进制按自己的默认继续服务；只有 `writable=True` 才追加 `--writable`。
- `scripts/acceptance/release_compatibility/schema.py`：仅候选 CLI 的 DDL 中途崩溃用例传 `writable=True`；旧二进制连接永不接收新 flag。

### F3 活动索引 swap 失败后的 best-effort 收尾清掉探测索引（`crates/store/src/turso_store/conversation/sessions/activity.rs`）

- 探测索引 `idx_sessions_activity_rebuild` 由自己的事务提交，swap 失败不会把它带回滚；此前收尾只恢复列表索引
  `idx_sessions_activity`，残留的探测索引会被之后每次会话写入口当作第二个活动索引维护。
- `restore_after_failed_swap` 现在两条语句都 best-effort：重建列表索引 + `DROP INDEX IF EXISTS` 探测索引（失败均
  warn），不掩盖 swap 的原始错误且幂等；两种半完成形态（仅探测 / 探测 + 列表索引）都收敛到只剩列表索引。
- 降级路径（重建因不可读行失败后 `append_message` / `update_session` 不刷活跃戳落地）的集成测试追加断言：失败的
  重建之后恰好只剩列表索引，后续写入同样不留探测索引。

### F4 `compact` 对所有 `Err` 清 compacting 状态（`crates/session/src/compaction/mod.rs`）

- `compact` 改为 `compact_inner` 的薄封装：内层在 summarize 前发出 `compacting conversation…`、只在成功路径清空；
  任何离开内层的 `Err`（summarizer 失败、取消、元数据持久化失败、harness 前置条件）都由封装补一次空 Status，
  前端不再在 runner 已放弃后继续渲染过期的 compacting 状态。
- `nothing to compact yet` 是 `Ok(None)`，不是错误，故意保留；成功路径状态恰好 raised / cleared 一次，封装不叠加。

### F5 两个 checkpoint 入口在 acquire lease 前拒绝非 UTF-8 路径

- `crates/database/src/maintenance.rs` `checkpoint`（TRUNCATE）：canonical key 的 `to_str()` 检查先于
  `try_acquire_exclusive`，错误文案点名 `database path must be UTF-8`。
- `crates/database/src/checkpoint/mod.rs` `handle_for`：同一检查先于 `acquire_shared` 与引擎构建，`standalone_opens`
  保持 0。
- 原因：lease 文件名由路径的有损渲染派生，若在 lease 之后才拒绝，会留下一个多余的 lease 文件；lease 模块自身也在
  建 lease 文件前拒绝不可命名路径，两个入口的前置检查让拒绝发生在任何 lease 文件出现之前。

### F6 写者存活判定的 error.chain 契约（`crates/database/src/maintenance_lock.rs`）

- `is_writers_active` 遍历 `error.chain()`，任一层是 `WritersActive` 即判定写者存活（经 `context` 包装或作为自定义
  错误 source 的冲突都能识别）；普通失败、甚至文案含 "locked" 的 `io::Error`（`PermissionDenied`）在同一 context 下都不算。
- `maintenance::writers_still_active` 只把真实冲突（链命中或 `WouldBlock`）映射为 "writers still active"，其余
  失败保留自身含义；契约由 `a_conflict_is_recognised_through_context_and_nothing_else_counts` 锁定。

### F7 `session_cmd` 死分支显式化；`ts -d` 锁冲突文案去重路径

- `crates/local/src/session_cmd.rs`：`reads_only`（list / show / export）提前返回后，只读子命令永远不会到达写者
  分支的 match，delete / import 也永远不会到达 `read_session`；两处改为显式 `unreachable!` 分支，不再重复处理。
- `crates/local/src/ts/actions.rs`：`ts -d` 对被存活写者持有的 store 的拒绝文案只命名一次 store 路径
  （`session store <db> is in use by a running session; … run opencoder ts -d <id> again`），底层冲突不再按
  context 层数重复路径；行保留、不取注册表写者。`ts -c` 不等待，该 store 的条目全部进 `kept_in_use`。

## 能力面

- 这些修复自身不新增 schema、不新增公开能力（无新的 CLI / API 入口、输入输出要求、依赖变化）；`storage serve` 的 `--writable` / `--read-only` 语义未变，F2 只改脚本侧如何传 flag。Ontology 无需变更。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| F1 注册表缺失：stopped legacy 会话按前缀命中（解析到导入行与其持久 workdir，注册表由导入创建且行含 store_dir / workdir） | `stopped_legacy_prefix_is_imported_before_it_is_resolved` | `crates/local/src/ts/resume_tests.rs` |
| F1 导入后仍歧义的前缀照样拒绝（同一歧义文案，不取首个匹配） | `ambiguous_legacy_prefix_is_still_refused_after_the_import` | `crates/local/src/ts/resume_tests.rs` |
| F2 rolling storage 默认连接显式 `--writable`、不带 `--read-only` | `test_default_connection_explicitly_takes_the_writable_side` | `scripts/platform/rolling_tests/test_storage_connection.py` |
| F2 readonly 连接保持 `--read-only`、不带 `--writable` | `test_readonly_connection_keeps_the_read_only_side` | `scripts/platform/rolling_tests/test_storage_connection.py` |
| F2 两侧 argv 仅最后一个 mode flag 不同 | `test_both_sides_differ_only_in_the_mode_flag` | `scripts/platform/rolling_tests/test_storage_connection.py` |
| F2 release compatibility `NativeSql` 默认不传 mode flag、仅候选 DDL 用例 `writable=True` | —（脚本改动，无独立用例） | `scripts/acceptance/release_compatibility/processes.py`、`scripts/acceptance/release_compatibility/schema.py` |
| F3 swap 失败收尾：探测索引已被自身事务提交，两种半完成形态都收敛到只剩 `idx_sessions_activity`，恢复的列表索引可回答行 | `failed_swap_cleanup_drops_the_committed_probe` | `crates/store/src/turso_store/conversation/sessions/activity.rs` |
| F3 真实损坏形态（索引失步 + 一行不可读）：重建失败后 append 落地、活跃戳不动、列表索引保留且无探测索引残留（后续写入同样） | `append_lands_without_activity_touch_when_the_index_cannot_be_rebuilt` | `crates/store/tests/session_activity_index_repair.rs` |
| F3 同一形态下 `update_session` 落地（摘要与序列写入）、同样只剩列表索引 | `update_session_lands_without_activity_touch_when_the_index_cannot_be_rebuilt` | `crates/store/tests/session_activity_index_repair.rs` |
| F4 summarizer 失败：`Err` 保留原错误、转录不变、不重试、状态 `compacting conversation…` → `""` | `compact_clears_the_compacting_status_when_the_summarizer_fails` | `crates/session/src/compaction/tests.rs` |
| F4 `Ok(None)` 保留 `nothing to compact yet`，不被守卫清空 | `compact_keeps_the_nothing_to_compact_notice` | `crates/session/src/compaction/tests.rs` |
| F4 取消：中断状态后状态被清空、转录不变、不产生合成摘要 | `compact_honors_cancel_and_leaves_messages_intact` | `crates/session/src/compaction/tests.rs` |
| F4 成功路径不变：状态恰好 raised / cleared 一次，守卫不叠加 | `compact_default_keeps_only_summary` | `crates/session/src/compaction/tests.rs` |
| F4 store 拒绝元数据持久化：外层 `persist compaction metadata`、链内原因可达、持久化标记、写入预算 3 次、summarizer 1 次、最后状态为空 | `compact_returns_err_when_store_rejects_metadata_persistence` | `crates/session/tests/compaction_error_propagation.rs` |
| F5 TRUNCATE checkpoint 在取 lease 前拒绝非 UTF-8 路径：点名原因、不算写者冲突、无 lease 文件、目录无新条目 | `truncate_checkpoint_refuses_a_non_utf8_path_before_any_lease_file` | `crates/database/tests/checkpoint.rs` |
| F5 standalone checkpoint 打开同样在取 lease / 建引擎前拒绝（`standalone_opens` 保持 0） | `handle_for_refuses_a_non_utf8_path_before_any_lease_file` | `crates/database/tests/checkpoint.rs` |
| F6 仅 lease 冲突算写者存活，且在 error.chain 任意层可识别；其它失败保持原义 | `a_conflict_is_recognised_through_context_and_nothing_else_counts` | `crates/database/tests/maintenance_lock.rs` |
| F7 `ts -d` 对存活写者持有的 store：单次尝试、可操作拒绝、路径只出现一次、不取注册表写者、行保留；`ts -c` 不等待并保留该 store 全部条目；持有者退出后两者照常完成 | `cleanup_and_delete_report_a_store_owned_by_a_live_writer` | `crates/local/src/ts/lock_tests.rs` |
| F7 `session` 子命令拆分：list / show / export 只读，delete / import 走写者 | `only_list_show_and_export_open_the_store_read_only` | `crates/local/src/session_cmd.rs` |

## 定向验证

- F2：`/usr/bin/python3 -m pytest scripts/platform/rolling_tests/test_storage_connection.py -v` → 3 passed
- F3：`cargo test -p opencoder-store --test session_activity_index_repair` → 4 passed
- F3：`cargo test -p opencoder-store --lib activity` → 15 passed
- F4：`cargo test -p opencoder-session --lib compaction` → 29 passed
- F4：`cargo test -p opencoder-session --test compaction_error_propagation` → 3 passed
- F5：`cargo test -p opencoder-database --test checkpoint` → 8 passed
- F6：`cargo test -p opencoder-database --test maintenance_lock a_conflict_is_recognised_through_context_and_nothing_else_counts`
  → 1 passed（只运行该用例，不代表 `maintenance_lock` 测试文件的其余用例）
- rustfmt：本轮 Rust 改动文件 `--check` 通过。
- clippy（`-D warnings`）通过的目标：store lib + `session_activity_index_repair`、session lib + `compaction_error_propagation`、
  database lib + `checkpoint` + `maintenance_lock`；范围限于这些目标，不是 crate 级 `--all-targets` 全量结果。
- 实施阶段定向结果（local 链路，在当时的树上通过，未在最终当前树重新运行）：`cargo test -p opencoder-local --lib ts::resume` → 9 passed、
  `cargo test -p opencoder-local --lib ts::actions::lock_tests` → 2 passed、`cargo test -p opencoder-local --lib session_cmd` → 14 passed。
- 测试范围：以上逐条独立执行，覆盖本次修改及其直接影响的 local / store / session / database 链路与 rolling storage 连接脚本；未运行全量测试。

## 未做（明确留待后续）

- Python 维护脚本与存活 owner 的写入协调（停 owner 或改走 owner 服务）仍是独立设计决策，本轮未处理。
- 远端任务禁 Fork 的 CLI / web 既有缺口未下沉，本轮未改。
- 手动 compact 持久化失败后的 TranscriptReset / UI 与模型上下文一致性未改。
- release compatibility 的 `schema.py` 改动没有独立自动化用例，由既有跨版本验收流程覆盖；本轮未执行该流程。

## Impact Surface

- 用户可感知：`ts -r <前缀>` 在尚无注册表时能命中已停止的 legacy 会话；compaction 失败 / 取消后 TUI 状态行不再卡在 `compacting conversation…`；`ts -d` 拒绝文案路径只出现一次。
- 存储：swap 失败后不再残留第二个活动索引；零 schema 变化。
- 运维脚本：rolling storage 连接显式声明打开侧；跨版本验收脚本对旧二进制保持零新 flag。
- 文件系统：非 UTF-8 路径被拒绝时不再留下 `<db>.maintenance.lock`。

## Related Docs

- [多写者 WAL / 维护租约](database-multi-writer-wal-and-maintenance-lease.md)（取代了单写者锁 / reader 意图方案）
- [compaction 瞬时零页降级持久化前序记录](compaction-transient-zero-page-degraded-persist.md)（F3 / F4 所在链路）
- [agents/foundation/store/index.md](../../../agents/foundation/store/index.md)、[agents/frontend/local/index.md](../../../agents/frontend/local/index.md)
