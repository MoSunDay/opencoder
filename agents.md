Commit: (working-tree, skill run-end 清除收敛 + bash_guard 换壳 shellguard + act chip task-plan 高亮)

# OpenCoder 逻辑地图

OpenCoder 是完全独立、从零实现的 Rust 原生编码代理。单二进制 `opencoder`，workspace 由 10 个 crate 组成。所有上层依赖 `Arc<dyn Store>` / `Arc<dyn ChatStream>` 两个抽象口子，存储与 LLM 后端均可替换。

## 模块索引

- [agents/shellguard](agents/shellguard/index.md) — sandbox 模式 shell 命令安全分类器（rippy MIT 衍生）：rable AST 级判定，释放集仅 `/tmp` + `/dev/null`，cwd 不释放，不可解析 fail-closed；被 session 的 bash_guard 薄适配消费。
- [agents/store](agents/store/index.md) — 持久化抽象层。`Store` trait + libsql 实现（WAL，本地嵌入）。所有 session/message/input/event/subagent 与 TODO 工作流持久化的唯一出口。未来可切其它 Rust SQLite 实现。
- [agents/llm](agents/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` trait + `MockChatClient` + token 估算器。
- [agents/session](agents/session/index.md) — 会话运行时核心：drain 主循环（steer/queue 提升）、工具注册（内建 + MCP + latent：ssh_pty/question 按 skill 解锁）、subagent 调度（explore/build + libsql 追踪）、sandbox 只读 bash 写拦截（bash_guard → shellguard 分类）、控制命令（/act、/sandbox、/act_clear_context）、压缩、resume、title 生成、cancel。
- [agents/core](agents/core/index.md) — 共享类型与 Config（模型/压缩/上下文窗口/small_model 全配置化）。
- [agents/web](agents/web/index.md) — axum HTTP + SSE 会话管理（prompt admit + 事件流 + 运行时切换 + interrupt）；全量 HMAC 请求签名中间件（token+timestamp+sig，±5min、重放 409）+ 编译期内嵌 React18+antd SPA（`spa/dist` 固定文件名 include_bytes! 白名单伺服）。
- [agents/cli](agents/cli/index.md) — clap 前端 + headless 运行时（run/tui/ts/daemon/config/models/session/todos/update/install-tools 子命令，`ts` 别名 `rs`；统一入口 `daemon --server | --client`，server/client/node 三子命令已收敛删除；--continue/--session/--fork/--model/--image；`session show --json` 深度观测面）。
- [agents/node](agents/node/index.md) — 分布式执行节点运行时（新 crate）：注册→心跳/claim 轮询→本地 Config+LLM 凭证跑任务→事件批量回传 server；纯出站 HTTP，无入站连接。
- [agents/tui](agents/tui/index.md) — ratatui 交互界面。
- [agents/todos](agents/todos/index.md) — 持久化 TODO 工作流运行时：父 Workflow Session 调度和验收，每个 TODO 使用独立 Primary Session 执行，支持依赖、并发、恢复、回退与可选 debug 投影。

## 关键抽象

- `Store` trait（`crates/store/src/store.rs`）：sessions/messages/session_inputs/session_events/subagent_tasks 的统一 CRUD 口子，是切换 SQLite 实现的唯一接缝。
- `ChatStream` trait（`crates/llm/src/stream.rs`）：`ChatClient`（真）与 `MockChatClient`（测试）共同实现，使 session 运行时可零 token 确定性测试。
- 节点任务状态机（`crates/store/src/libsql_store/node_state.rs::transition_allowed`）：pending→running→done|error|cancelled，running/pending 经 cancelling 收束；终态冻结。失联收束：`Store::converge_lost_node_tasks(now_ms, stale_ms)` 单事务把「心跳超 stale 且状态 ∈ running/cancelling」的僵尸任务置 `error("node lost")`，由 `GET /api/nodes` 读路径机会式触发并对每个收束任务广播 `error` 终帧——worker 零代码变更即可重新领取。claim 靠 BEGIN IMMEDIATE 内条件 UPDATE CAS（不用 RETURNING），FIFO 以 `(created_at, rowid)` 定序——同毫秒 ULID 无单调性不可作 tiebreak。
- drain 语义（`crates/session/src/runner/mod.rs::run_loop`）：每个 turn 边界提升 steer；idle 边界逐条消费 queue，单 run FIFO 排空至 Done。doom-loop 守卫（`DOOM_THRESHOLD=20`，定义于 `runner/event.rs`）：滑动窗口内 20 个相同 `name:input` 工具签名 → Error + Err 终止 run。
- 请求签名协议（`crates/core/src/auth_sig.rs`）：canonical 四行串 + HMAC-SHA256(token) 小写 hex，头 `x-sig-timestamp`/`x-sig`；`crates/web/src/auth_sig_mw.rs` 负责 ±5min 窗口、进程内重放缓存（同签 409）、2MB body 上限（413），豁免 `/`、`/static/*`、`/api/time`、`/favicon.ico`。SPA 与 worker 节点共用此协议。

业务能力见 [features/index.md](features/index.md)。

## 仓库规则

开发必须遵循 [rules/](rules/) 目录下的规则：

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有对应测试用例
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 每轮迭代结束前全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层规范（unit / integration / e2e）
