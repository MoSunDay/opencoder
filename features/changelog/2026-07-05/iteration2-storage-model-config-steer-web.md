Commit: (working-tree, pre-initial-commit)

# 迭代二：存储迁移 / 模型压缩配置化 / steer-followup / 会话恢复 / Web SSE

## Context
OpenCoder 迭代一已建立 7-crate workspace 与基础 agent 运行时，但存储为无元数据的 JSONL、仅 web 层持久化、无恢复入口、模型/压缩配置僵化、web prompt 同步阻塞。迭代二对齐 opencode 的关键能力并补齐测试金字塔（90% 覆盖目标 + glm5.2 e2e）。

## Change Summary
- **存储层（P0）**：新增 `Store` trait（`crates/store/src/store.rs`）作为切换接缝；libsql 0.9.30 嵌入实现（WAL、单连接缓存以支持 `:memory:`）；schema 含 sessions/messages/session_inputs/session_events + schema_version；JSONL 一次性导入；事务保证 all-or-nothing。
- **模型/压缩配置化（P1）**：Config 增 `small_model`/`context_limit`/`max_tokens`；CompactionConfig 增 `prune`/`buffer`，`reserved` 接通（预算 = `min(threshold, context_limit - reserved)`）；新增 token 估算器（chars/4）使压缩首轮即可触发；摘要与 title 走 small_model；CLI 加 `--small-model`/`config show`/`models`。
- **会话恢复（P3）**：`SessionState.record()` 把 runner 持久化贯穿所有入口（不再仅 web）；`resume()` 从 Store 重建并忠实原 model/agent；CLI `--resume/--session/--continue/--fork` + `session list/show/delete`；`generate_title` small_model 异步。
- **steer/followup（P2）**：run_loop 改造为 drain 语义——turn 边界 `claim_steers`（重置 step=1）、idle `claim_next_queue`（恰好一条）；新增 `claim_next_queue` 原子操作；空 prompt = drain 模式。
- **Web SSE（P4）**：axum 路由重构——`POST /prompt` admit 即返回（非阻塞）、`GET /events` replay+live（BroadcastStream）、`POST /agent|/model|/interrupt`；`SessionHandle` + `HandleMap` + 后台 drain 任务。
- **可中断**：SessionState 增 `cancel: Option<CancellationToken>`，run_loop 每轮顶部检查（web interrupt）。
- **性能（P5）**：release 启动 ~6ms（opencode Bun ~1489ms，~250×）；store 0.031ms/append、2.4ms/load1000、0.95ms/list200。见 [docs/perf.md](../../../../docs/perf.md)。
- **配置发现**：`Config::load` 由「首个存在即用」改为「全部候选深度合并」（global base → project override），新增 `~/.opencoder/config.json` 作为二进制自有配置主目录——`opencoder` 从任意目录直接执行，全局提供 provider+key、项目仅覆盖 model。
- **测试基建**：`ChatStream` trait + `MockChatClient`（FIFO 脚本回放 + 请求录制）使 session/web 测试零 token 确定性。
- **e2e**：`scripts/e2e-glm.sh` 真 glm5.2 写贪吃蛇/雷霆战机，捕获并修复 `max_tokens` 过小导致 `finish_reason=length` 截断工具调用的真实 bug。

## Impact Surface
- 新增 crate 依赖：libsql、tokio-util（CancellationToken）、tokio-stream sync feature（web BroadcastStream）。
- `SessionState.client` 类型由 `ChatClient` 改为 `Arc<dyn ChatStream>`（破坏性，所有调用方已改）。
- 存储后端由 JSONL 迁至 libsql；旧 `.jsonl` 经 `import_jsonl_dir` 导入。
- workspace 测试从 0 增至 46（store 14 / llm 4 / core 5 / session 16 / web 6 / perf 3 - 重叠计数），clippy `-D warnings` 全绿。

## Notes / Compatibility
- libsql `:memory:` 必须缓存单连接（每次 `db.connect()` 返回独立空库）。
- libsql `PRAGMA journal_mode=WAL` 返回行，必须用 `query`+drain，`execute` 报错。
- 小模型 id（去 provider 前缀）必须用于请求体；`small_model_or_primary()` 现返回 id（修复了 glm5.2 "模型不存在" 报错）。
- session_inputs 的 FK 要求 session 行先存在（web `ensure_session_row` / 测试 `seed_session`）；sessions 表 INSERT 用 `INSERT OR IGNORE` 保证幂等。
- compaction 当前仅在单次运行内生效；持久化的是未压缩完整转录（resume 重载全量，压缩按需重触发）——无数据丢失。

## Related Docs
- [agents/store](../../../agents/store/index.md)
- [agents/session](../../../agents/session/index.md)
- [agents/web](../../../agents/web/index.md)
- [docs/perf.md](../../../../docs/perf.md)
