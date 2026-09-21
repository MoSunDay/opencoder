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
- [agents/brain](agents/brain/index.md) — 能力库、版本化本体计划、有限调度状态机、v4 分层能力画布与兼容路由。
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

- [tests/operator_e2e/](tests/operator_e2e/main.rs) — O1–O5：operator 创建→drain→idle、relay SSE 守卫、角色门禁（operator/agent 提交与指令放行、dag/team 403）、interrupt→cancelled + 重启恢复、`kind=agent` 会话（how_append 注入、output_text/output_json、`?kind=agent` 泳道列表且不漏入 operator 泳道）（真二进制 + 共享 LLM 桩）；O6：agent 卡 `run_mode` 分派（`agent_sandbox.rs`：无沙箱运行时准入 fail-closed、runc 全回合契约 + follow-up 二轮续会话、operator 卡保持宿主循环，无 runc 时 SKIP 运行段）。
- [tests/dag_e2e/](tests/dag_e2e/main.rs) — D1–D3/D5：spec 保存/dispatch 流、wasm 版本池（wat 现场编译）、cancel/失败折叠、runc preflight（无 runc 时 SKIP 运行段）、structured_output（agent 步围栏/裸 JSON 两种形态 output.json 非 null）、agent runc 沙箱（`agent_runc.rs`：真实容器内 session 全链路，无 runc 时 SKIP）、code-review 门禁 DAG（`code_review.rs`：9 步 pass/blocked 两线 + compat input 透传 + 工单 not_required/created）、review 门禁三件套（`review_dags/`：R1 启动 seed、R2 host imports+fail-closed、R3/R5 池模块 dispatch seeded def、R4 六段 agent 链（api-impact→client 客户端分支汇入 verdict）上下文传递+五字段索引）。
- [tests/todos_e2e/](tests/todos_e2e/main.rs) — T1–T3：模板→运行→完成、interrupt→节点重启→resume→done、子 LLM 失败→todo failed + workflow suspended（父决策重试链共 5 请求）。
- [tests/team_e2e/](tests/team_e2e/main.rs) — M1：能力注册/绑定→pinned 定义冻结 capabilities→member prompt 能力前缀→四段 chat 决策→finished topic + 1-based turn 台账。
- [tests/brain_e2e/](tests/brain_e2e/main.rs) — B1/B2：plan-defs 固化 v1→fixed run 经子会话+收据路由 completed→重放 202 零调用；旁路 409、异样 receipt→blocked→cancel→终态命令拒绝。子会话标题 pass 与 route 竞态，stub 按内容判别。
- [tests/brain_layered_e2e/](tests/brain_layered_e2e/main.rs) — L1–L3：v4 分层画布准入与读取面（裸 `/api/executions` 提 Brain 仍 409、`schema_version` 非 3|4 → 409 且零模型调用、非法画布 → 400 不建运行；`/layered` 视图与 `/layered/rounds/:round` 键与拒绝面，v4 运行走 v3 `/view` → 409、v3 走 `/layered` → 404）、真实 runc 画布跑到 completed（层屏障先等待再放行、逐层事件序 `run_created→layer_started→node_dispatched→operation_terminal→layer_barrier_reached→…→run_completed`、子执行 `brain_layered` 绑定与 `scheduler_output`、CLI 与模型调用计数）。无 runc 时 SKIP 运行段。
- [tests/support/](tests/support/mod.rs) — `llm_stub`（FIFO/Hold/Fail/**Dynamic 请求感知**流式桩 + `/embeddings` canned、`Script: Clone`）、`http_util`（Bearer JSON + SSE 解析）、`fleet_proc`（fleet 拉起/RAII/kill+respawn）；`tests/running_mode_switch_e2e.rs` 与 todos/team/brain e2e 共用。

## 仓库规则

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有测试
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 迭代结束全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层（unit/integration/e2e）
