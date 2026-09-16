Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# OpenCoder 逻辑地图

Rust 原生编码代理 workspace：`opencoder`（本地 CLI/TUI）、`opencode-cli`（远程管理）、`opencoder-server`（控制面）、`opencoder-agent`（节点执行）。

抽象口子：`Arc<dyn Store>`、`Arc<dyn ChatStream>`。细节见各模块索引，代码是最终事实。

## 模块索引

- [agents/core](agents/core/index.md) — 共享类型与 Config。
- [agents/llm](agents/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` + `MockChatClient` + token 估算。
- [agents/store](agents/store/index.md) — `Store` trait + libsql（WAL）持久化层。
- [agents/session](agents/session/index.md) — 会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
- [agents/shellguard](agents/shellguard/index.md) — shell 安全分类器：rable AST 判定、释放集仅 `/tmp`+`/dev/null`、fail-closed。
- [agents/tui](agents/tui/index.md) — ratatui 交互界面。
- [agents/local](agents/local/index.md) — 本地 CLI 前端：参数解析、headless、tmux 会话入口。
- [agents/web](agents/web/index.md) — axum HTTP + SSE 会话管理 + 内嵌 SPA。
- [agents/dag](agents/dag/index.md) — DAG 纯域 + 线协议（DTO LOCKED）。
- [agents/dag-wasm](agents/dag-wasm/index.md) — DAG wasm 模块版本池：发布、NFS 导出、节点冻结分发。
- [agents/dag-runtime](agents/dag-runtime/index.md) — 节点侧 DAG 调度执行；server 不链接。
- [agents/todos](agents/todos/index.md) — 持久化 TODO 工作流：每 TODO 独立 Primary Session。
- [agents/project](agents/project/index.md) — 项目跟踪：goal→milestone→todo，`ProjectStore` 接缝。
- [agents/brain](agents/brain/index.md) — 能力库、版本化本体计划、有限调度状态机与兼容路由。
- [agents/agents](agents/agents/index.md) — 版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。
- [agents/team](agents/team/index.md) — 团队目录与消息扇出运行时。
- [agents/control](agents/control/index.md) — 平台控制面：节点调度、五字段执行索引。
- [agents/worker](agents/worker/index.md) — 节点执行面：接受/恢复、资源快照、操作适配。
- [agents/node](agents/node/index.md) — 出站 WebSocket：注册、心跳、RPC。
- [agents/server](agents/server/index.md) — 版本 Server 与独立只读资源服务入口。
- [agents/agent](agents/agent/index.md) — 稳定 Host、独立版本 Runtime 与兼容节点入口。
- [agents/ctl](agents/ctl/index.md) — `opencoder-cli`：Server API + Bearer + 退出码约定。

业务能力见 [features/index.md](features/index.md)。

## 根包进程级 e2e 套件（layer-2，随 cargo test 运行）

- [tests/operator_e2e/](tests/operator_e2e/main.rs) — O1–O4：operator 创建→drain→idle、relay SSE 守卫、角色门禁 403、interrupt→cancelled + 重启恢复（真二进制 + 共享 LLM 桩）。
- [tests/dag_e2e/](tests/dag_e2e/main.rs) — D1–D3/D5：spec 保存/dispatch 流、wasm 版本池（wat 现场编译）、cancel/失败折叠、runc preflight（无 runc 时 SKIP 运行段）。
- [tests/support/](tests/support/mod.rs) — `llm_stub`（FIFO/Hold/Fail 流式桩）、`http_util`（Bearer JSON + SSE 解析）、`fleet_proc`（fleet 拉起/RAII）；`tests/running_mode_switch_e2e.rs` 已复用 `llm_stub`。

## 仓库规则

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有测试
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 迭代结束全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层（unit/integration/e2e）
