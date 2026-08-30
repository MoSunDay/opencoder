# store 冷启动 fsync 风暴修复：pragma 顺序不变量 + bootstrap 单事务 + 启动阶段计时

## 背景

用户报启动有概率卡数十秒到分钟级。实测钉死：全新 workdir 建库路径 `session list` 在 I/O 风暴窗口（load 97+，cargo 风暴 + 多 TUI 并存）耗时 32.7s—77.8s，strace 显示建库发出 8 次 fsync、单次 0.26—1.2s、风暴中 >10s（存储层 ZFS rpool `sync=standard`，fsync = ZIL 刷盘）。根因链：

1. `PRAGMAS` 顺序为 busy_timeout → journal_mode=WAL → synchronous=NORMAL。全新库（无 header）切入 WAL 时做 header 初始化并 fsync，此刻 `synchronous` 还是默认 **FULL**——既定的 NORMAL 策略对建库最贵的一步完全不生效。
2. `bootstrap` 17 条 DDL 各自隐式提交，写放大 ×17，migrate 半途失败还会留下半套 schema。
3. store 打开/建库串在 TUI 首帧之前，且启动无任何阶段计时，慢启动无法事后归因。

## 变更

### `crates/store/src/libsql_store/schema.rs`
- **PRAGMA 顺序不变量**：`busy_timeout` 第一，`synchronous=NORMAL` 提到 `journal_mode=WAL` 之前，WAL 切换 fsync 从此落在 NORMAL 下；附顺序说明注释。
- **bootstrap 单事务化**：薄壳 `run_tx(conn, "BEGIN IMMEDIATE", bootstrap_tx)`，17 次隐式提交 → 1 次；任一步失败整体回滚（migrate 原子性增强）；`BEGIN IMMEDIATE` + 已生效 busy_timeout 使并发打开在写锁上排队。语句顺序原样保留（迁移后索引批依赖 migrate 产出的列）。
- `write_version`（无事务 DELETE+INSERT 体）从 `set_version` 拆出；`set_version` 保留原事务语义供既有单测。
- 新增单测 `pragma_order_synchronous_precedes_journal_wal`：断言 busy_timeout < synchronous < journal_mode 顺序（生效值断言抓不住本 bug——顺序错时最终值仍是 NORMAL）。

### `crates/store/src/libsql_store/mod.rs`
- `LibsqlStore::open` 四阶段计时（build/pragmas/bootstrap/checkpoint），info 汇总 + 任一阶段 >1s 发 `WARN slow store open`（含 slowest_stage/slowest_ms）。TUI/run/ts/web/daemon 全部入口自动受益。

### `crates/store/src/libsql_store/mod.rs`（风暴复测追加）
- **fresh 建库跳过 bootstrap 后的 TRUNCATE checkpoint**（`existed` 判定）：同风暴窗口实测 checkpoint 段吃掉 46s/82s，是残留第一大户；WAL 本就是真相源，`wal_autocheckpoint` 会后续收敛，跳过不影响正确性（重开幂等/integrity_check 契约覆盖该路径）。该路径不再产生 checkpoint fsync。

### `crates/tui/src/`（首帧与启动分段观测）
- 新模块 `boot_clock.rs`：`mark()` 一次性记录进程起点，`note_first_frame()` once-guard 记录 mark→首帧毫秒（>1s 告警）；纯函数 `frame_log_ms`/`is_slow_frame` + 4 个单测。
- `render.rs` 主帧函数首行挂 `note_first_frame()`（+1 行，790/800）。
- `app_bootstrap.rs` 分段计时（config+client / store / ts mirror / session resume/create / terminal enter），`run_app` 前 info 汇总 + >1s WARN 最慢段（+27 行）。

## 数据（本机即生产等价环境，跨 agent 共租负载下的同窗口 A/B）

| 项 | 旧二进制 | 新二进制（pragma 序 + 单事务） | 新二进制（+fresh 跳过 checkpoint） |
|---|---|---|---|
| 自然风暴 fresh `session list`（样本1，load 97） | 77.79s | **1.36s（57×）** | — |
| 自然风暴（样本2） | 133.4s | 87.83s → P0-3 打点拆出 `pragma 35s + checkpoint 46s + bootstrap 1.1s` | — |
| dd 同步写风暴（×8 oflag=sync） | — | — | **6.12s**（`checkpoint_ms=0` 证实生效） |

- 建库路径 fsync 次数：8 → 7 →（skip 后 fresh 路径 checkpoint 段归零）。
- 结论：FULL-synced 17 次隐式 DDL 提交坍缩为 1 次 NORMAL 提交（bootstrap 段从主犯降为 ~1s）；剩余结构性 fsync = WAL 切换（必须保留）+ fresh 路径已消除的 checkpoint。
- 观测即证据：P0-3 上线首日即完成两次归因（`slowest_stage=checkpoint 2030ms` 与 `35s+46s` 拆分），检查跳过决策直接由该数据驱动。
- 共租干扰澄清（如实记录）：共租方持续同步写（rpool ~7MB/s）压 ZIL 队列时，load 5 也观测到单次 fsync >100s 的干扰窗——此类窗口与本修复无关，应由 tui.log 阶段 WARN 长期观察归因。
- **干净安静窗口最终值：fresh workdir `session list` 80ms（store open total 50ms，checkpoint_ms=0），<300ms 目标达标**；既有 workdir 基线 44ms 不受影响。
- headless 路径日志落 `/tmp/opencoder-tui.log`（fallback），TUI 落 `~/.local/share/opencoder/tui.log`。

## 测试清单

| 层 | 契约 | 位置 |
|---|---|---|
| 单元 | pragma 顺序不变量（synchronous 先于 WAL） | `crates/store/src/libsql_store/schema.rs::tests` |
| 单元 | set_version 单行原子替换（既有，未动） | 同上 |
| 集成 | open 后 synchronous=1、journal_mode=wal | `crates/store/tests/schema_bootstrap.rs` |
| 集成 | fresh open → 建会话 → 重开×2 幂等：会话存活、version 单行、integrity_check ok | 同上 |
| 集成 | 首 store 存活时同路径二次 open（并发打开，BEGIN IMMEDIATE 排队） | 同上 |
| 单元 | boot_clock：未 mark no-op / 只记一次 / 饱和 / 阈值告警 | `crates/tui/src/boot_clock.rs` |
| 全量 | 分包回归（共租锁竞争下分包执行）：store 全套 / tui 26 / session 90 / core+llm+shellguard 22 / cli+todos 21 / web+node 13 套件全绿 | 2026-08-30 本机 |
| e2e | daemon_smoke 0.29s、nodes_smoke_proc 31.7s 单独复跑绿（首轮全量中被共租 e2e/风暴干扰失败，复跑证实为环境 flake） | `crates/cli/tests/`（daemon）、根包 |
| clippy | `cargo clippy -p opencoder-store -p opencoder-tui --all-targets` | 0 warning |
| 终验 | shellguard 收口后提交前全量：`cargo test --workspace --no-fail-fast` 3689 passed / 0 failed + `cargo clippy --workspace --all-targets -- -D warnings` 0 warning + `cargo build --workspace` 干净 | 2026-08-30 本机 |

## Impact Surface

- 所有经 `LibsqlStore::open` / `TsRegistry::open` 的入口（TUI、run、ts、web、daemon、node）共用本修复；无 schema 变更、无迁移，仅 pragma 顺序与事务边界；既有库读回不受影响。
- 不影响：cargo 回归门禁、历史 workdir 数据、ZFS 全局配置。

## Related Docs

- [agents/store](../../../agents/store/index.md)
