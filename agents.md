Commit: (working-tree, pre-initial-commit)

# OpenCoder 逻辑地图

OpenCoder 是完全独立、从零实现的 Rust 原生编码代理。单二进制 `opencoder`，workspace 由 7 个 crate 组成。所有上层依赖 `Arc<dyn Store>` / `Arc<dyn ChatStream>` 两个抽象口子，存储与 LLM 后端均可替换。

## 模块索引

- [agents/store](agents/store/index.md) — 持久化抽象层。`Store` trait + libsql 实现（WAL，本地嵌入）。所有 session/message/input/event 持久化的唯一出口。未来可切其它 Rust SQLite 实现。
- [agents/llm](agents/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` trait + `MockChatClient` + token 估算器。
- [agents/session](agents/session/index.md) — 会话运行时核心：drain 主循环（steer/queue 提升）、工具注册、subagent 调度（explore/build + libsql 追踪）、plan 模式 bash 写拦截（bash_guard）、压缩、resume、title 生成、cancel。
- [agents/core](agents/core/index.md) — 共享类型与 Config（模型/压缩/上下文窗口/small_model 全配置化）。
- [agents/web](agents/web/index.md) — axum HTTP + SSE 会话管理（prompt admit + 事件流 + 运行时切换 + interrupt）。
- agents/cli — clap 前端（run/tui/serve/config/models/session 子命令，--continue/--session/--fork/--small-model）。
- agents/tui — ratatui 交互界面。

## 关键抽象

- `Store` trait（`crates/store/src/store.rs`）：sessions/messages/session_inputs/session_events/subagent_tasks 的统一 CRUD 口子，是切换 SQLite 实现的唯一接缝。
- `ChatStream` trait（`crates/llm/src/stream.rs`）：`ChatClient`（真）与 `MockChatClient`（测试）共同实现，使 session 运行时可零 token 确定性测试。
- drain 语义（`crates/session/src/runner.rs::run_loop`）：每个 turn 边界提升 steer；idle 时消费恰好一条 queue。doom-loop 守卫（`DOOM_THRESHOLD=3`）打破连续空 turn 循环。

业务能力见 [features/index.md](features/index.md)。

## 仓库规则

开发必须遵循 [rules/](rules/) 目录下的规则：

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有对应测试用例
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 每轮迭代结束前全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层规范（unit / integration / e2e）
