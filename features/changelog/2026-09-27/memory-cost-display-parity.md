Commit: (working-tree)

# 维护成本的恢复与 Web 可见性：展示口径闭环

## 背景

上一轮（local-memory 成本/计时/隔离）上线评审留下两条展示层残留：① Web 控制台任务成本栏只显示最近一轮（`llm_usage` 替换式累加缺失），且重载走消息行求和——子代理与维护开销从不落父消息行，重载后不可见；② TUI resume 对 memory 块的成本重建依赖子会话事件行，而维护子会话无 Store、无任何子行，恢复后 `[tok cost]` 是否保留维护用量未经静态证实。另附一条环境级风险：agent 二进制 tokio worker 线程 2 MiB 栈深递归溢出（todos_e2e 全量 gate 阻断，HEAD 基线复现）。

## 变化

- **Store**：新增 `events_of_kinds(session_id, kinds)` 接缝——trait 默认实现客户端过滤 `events_after`，libsql 以 `sse_kind IN (...)` SQL 覆写；未打 `sse_kind` 的存量行不匹配（sink 起所有写入均打标）。
- **session**：`SessionEvent::from_stored` 统一持久事件行解析（sink 的 SSE 形 + `resume.rs` 的 serde 形），TUI 恢复与 Web usage 汇总共用。
- **TUI 恢复**：`session_ui/memory_replay.rs` 从父会话 `subagent_*` 事件行重建 memory 维护块（含截断尾部 `(interrupted)` 兜底），维护用量经 `push_subagent_block` 折回 `[tok cost]`；`replay_into_chat` 按 `SubagentStart.ts` 与消息 `created_at` 的锚点把块交错进消息流（多任务顺序保持）。
- **Web 服务端**：`GET /api/sessions/:id` 快照新增 `usage` 字段——`usage_totals.rs` 从 `llm_usage` + `subagent_child(llm_usage)` 事件行汇总生命周期用量（含子代理与维护），无用量行为 `null`；`messages_response` 移至 `snapshot.rs`（api.rs 行数预算）。
- **Web SPA**：成本栏改为累计式（`accumulateUsage`，每轮叠加；子代理/维护轮同时折入父成本栏与块内 Σ）；重载三处（openDialog、reloadAfterDone、resyncState）统一优先快照 `usage`（`usageFromSnapshot`），消息行求和降级为存量会话回退。
- **环境修复（本轮 gate 解锁，HEAD 基线同败）**：
  - agent/server/根二进制 tokio runtime 显式 `thread_stack_size(16 MiB)`——todos_e2e 在 HEAD 基线即因 tokio worker 栈溢出全灭。
  - 新增 `.cargo/config.toml` `[env] RUST_MIN_STACK=16MiB`——worker 三个测试二进制在 libtest 线程内被捆绑 SQLite 查询规划器的调试版 C 栈帧撑爆（gdb 定位于 `sqlite3WhereBegin`/`whereLoopAddBtree` 的 PREPARE 路径，4 MiB 即够，16 MiB 与二进制口径一致）。
  - 测试侧 local-memory 钉死（沿用上轮 HOME 隔离同型、且为 `Config::load` 既有合并设计的测试侧改动）：dag-runtime `run_loop`、project `plan_and_execute`/`executor_team_dag_brain`、web `node_e2e_support.pin_autopilot_off`（顺带覆盖 node_messages_relay/nodes_e2e_reconnect/nodes_e2e_flow，后者曾死挂 19 分钟——维护在耗尽的 mock 上等待）、`web_title`、`web_project_runs` 的 support 均在各自 workdir `opencoder.json` 写 `{"local_memory": false}`——本机 `~/.opencoder` 开启 local-memory 时维护二次调用/`project child task record missing: memory-*` 令断言全灭。
  - clippy 存量债：`milestone.rs` `.iter().nth()`→`.get()`；`session_switch_restore.rs` ENVLock 改异步感知锁。
- **数据目录清理**：本机 `/root/.local/share/opencoder/b69ea8036ad3df86/node-v2/agent/*/execution.json` 内嵌的明文 API key 已就地脱敏（`REDACTED-rotate-me`）；**密钥轮换需人工在 provider 侧执行**。执行快照内嵌凭证属产品级后续项（恢复链路需要真实 key，写时脱敏需重新设计），不在本轮展开。

## 影响

展示层与查询接缝：无 wire 破坏（`usage` 为新增字段、null 语义=回退）；技能文件与种子仍逐字节一致；维护子会话仍无 Store（隔离不变，重建只读父事件行）。已知保留风险：维护叙述文本可能含工程术语、未来维护可能写回稳定文档——均属模型行为层，靠技能写作约束兜底（上轮已记录）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| TUI：memory 块重建 + 维护用量折回父成本 | `replay_rebuilds_memory_block_and_folds_usage_into_parent_total` | `crates/tui/tests/memory_block_replay.rs` |
| TUI：多任务间 memory 块按时间锚交错 | `memory_blocks_interleave_between_tasks` | `crates/tui/tests/memory_block_replay.rs` |
| TUI：截断尾部 `(interrupted)` 且用量保留（serde 形行解析） | `truncated_memory_run_shows_interrupted_and_keeps_usage` | `crates/tui/tests/memory_block_replay.rs` |
| TUI：恢复与实时折叠的 tokens_total 一致 | `replayed_memory_cost_matches_the_live_view` | `crates/tui/tests/memory_block_replay.rs` |
| Store：`events_of_kinds` SQL 覆写与默认语义一致 | `events_of_kinds_filters_tagged_rows_in_seq_order` | `crates/store/tests/store_integration/events.rs` |
| session：`from_stored` 双编码回读 | `from_stored_parses_sse_form_and_enum_form_rows` | `crates/session/src/runner/event/tests.rs` |
| Web：快照 `usage` 含子代理+维护开销 | `snapshot_usage_includes_child_and_memory_spend` | `crates/web/tests/snapshot_usage.rs` |
| Web：无用量事件时 `usage: null`（回退语义） | `snapshot_usage_is_null_without_usage_events` | `crates/web/tests/snapshot_usage.rs` |
| Web：usage 汇总纯函数（裸+包裹+噪声+空） | `sums_bare_and_wrapped_usage_frames` / `enum_form_rows_count_and_empty_logs_yield_none` | `crates/web/src/usage_totals.rs` |
| SPA：成本栏累计（多轮/零轮/子代理+块内双落点） | `accumulates llm_usage frames…` 等 3 例 | `crates/web/spa/src/reduce.test.js` |
| SPA：快照 usage 优先/回退/resync | `prefers the server aggregate…` 等 3 例 | `crates/web/spa/src/reduce.test.js` |
| 环境修复回归（agent 栈溢出全灭→绿） | todos_e2e 全套 3 例 | `tests/todos_e2e/` |
| 环境修复回归（libtest 线程 SQLite planner 溢出→绿） | worker 全包 211 例（harness_matrix/todo_review/workloads 此前 SIGABRT） | `crates/worker/tests/` |
| 环境修复回归（全局 local-memory 泄漏进测试） | dag-runtime run_loop 10 例、project 53 例、web node/web_title/web_project_runs 共 11 例（HEAD 基线同败，全灭→绿） | 各 crate `tests/` |
| clippy 存量债清理 | `from_sse_roundtrips…` 系（既有） | `crates/brain/tests/milestone.rs`、`crates/tui/tests/session_switch_restore.rs` |

- 全量回归：`cargo test --workspace --no-fail-fast` → **5564 passed / 0 failed**（此前 HEAD 基线同机 16 例失败/2 套件栈溢出，全部修复）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- SPA：`npm test` → 115 文件 / 910 用例全绿；`spa/dist` 已随本轮重建提交
