Commit: af71944

# OpenCoder 逻辑地图

OpenCoder 是完全独立、从零实现的 Rust 原生编码代理。单二进制 `opencoder`，workspace 由多个 crate 组成。所有上层依赖 `Arc<dyn Store>` / `Arc<dyn ChatStream>` 两个抽象口子，存储与 LLM 后端均可替换。

## 模块索引

- [agents/shellguard](agents/shellguard/index.md) — sandbox 模式 shell 命令安全分类器（rippy MIT 衍生）：rable AST 级判定，释放集仅 `/tmp` + `/dev/null`，cwd 不释放，不可解析 fail-closed；被 session 的 bash_guard 薄适配消费。
- [agents/store](agents/store/index.md) — 持久化抽象层。`Store` trait + libsql 实现（WAL，本地嵌入）。所有 session/message/input/event/subagent 与 TODO 工作流持久化的唯一出口。未来可切其它 Rust SQLite 实现。
- [agents/llm](agents/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` trait + `MockChatClient` + token 估算器。
- [agents/session](agents/session/index.md) — 会话运行时核心：drain 主循环（steer/queue 提升）、工具注册（内建 + MCP + latent：ssh_pty/question 按 skill 解锁）、subagent 调度（explore/build + libsql 追踪）、plan 只读 bash 写拦截（bash_guard → shellguard 分类核，cwd 对齐）、控制命令（/act、/plan、/act_clear_context）、压缩、resume、title 生成、cancel。
- [agents/core](agents/core/index.md) — 共享类型与 Config（模型/压缩/上下文窗口/small_model 全配置化）。
- [agents/web](agents/web/index.md) — axum HTTP + SSE 会话管理（prompt admit + 事件流 + 运行时切换 + interrupt）；全量 HMAC 请求签名中间件（token+timestamp+sig，±5min、重放 409）+ 编译期内嵌 React18+antd SPA（`spa/dist` 固定文件名 include_bytes! 白名单伺服）。
- [agents/cli](agents/cli/index.md) — clap 前端 + headless 运行时（run/tui/ts/daemon/config/models/session/todos/update/install-tools 子命令，`ts` 别名 `rs`；`daemon` 子命令仅打印迁移指引（P0 拆分：server→`opencoder-server`、node→`opencoder-agent`）；--continue/--session/--fork/--model/--image；`session show --json` 深度观测面）。
- [agents/node](agents/node/index.md) — 分布式执行节点运行时（新 crate）：注册→心跳/claim 轮询→本地 Config+LLM 凭证跑任务→事件批量回传 server；纯出站 HTTP，无入站连接；`DagHook` 扩展点（claim/execute，idle 轮询 DAG run 单活跃串行执行，取消经心跳 `cancel_run_ids` 捎带）。
- [agents/dag](agents/dag/index.md) — DAG workflow 纯域 + 线协议（spec/validate、ready_steps/run_outcome/render_context、`/workflow/<run_id>/<step>/` 工件契约、protocol DTO LOCKED）：server 校验与节点执行共享的唯一契约。
- [agents/dag-runtime](agents/dag-runtime/index.md) — 节点侧 DAG 执行运行时（新 crate）：JoinSet 有界调度 + 事件批量上行 + agent step（真 session）/python step（内嵌 RustPython VM 或 runc OCI 沙箱）+ `prepare-rootfs` 脚手架。仅 `opencoder-agent` 链接。
- [agents/server](agents/server/index.md) — `opencoder-server` 二进制：web 服务薄入口，不链接 VM/runc。
- [agents/agent](agents/agent/index.md) — `opencoder-agent` 二进制：fleet worker（node 运行时 + DAG hook）。
- [agents/tui](agents/tui/index.md) — ratatui 交互界面。
- [agents/todos](agents/todos/index.md) — 持久化 TODO 工作流运行时：父 Workflow Session 调度和验收，每个 TODO 使用独立 Primary Session 执行，支持依赖、并发、恢复、回退与可选 debug 投影。
- [agents/project](agents/project/index.md) — 用户策展的项目跟踪运行时（新 crate）：goal→milestone→todo 三级，todo 走「草稿→plan agent 生成方案→act agent 执行」生命周期，执行 resume 同一会话持续推进，`project_todo_runs` 版本留痕可取消；复用 session 直驱范式（非 todos 编排），项目数据走独立 `ProjectStore` 接缝（默认 libsql 同实例，feature-gate 可选 mysql/starrocks）。
- [agents/brain](agents/brain/index.md) — 项目目标/能力库（新 crate）：能力条目（类型/描述/输入/输出/工程输入）录入 + 语义向量检索；embed 走 OpenAI 兼容 `/embeddings`（`ChatStream::embed`），向量存 libsql bundled `vector_distance_cos`（BLOB=LE f32），store schema v15 三张 brain 表，web `/api/brain/*` + SPA「项目目标」tab。
- [agents/agents](agents/agents/index.md) — 版本化自定义 Agent（opencoder-agents crate）：共享池（prompts/skills/tools/memory/<名>/v{n}，版本只增、回滚=切指针）+ 引用卡（agents/<名>/meta.json 四字段引用，多 agent 共享同一资源）+ active marker；读路径在 core `agent::{meta,resource,compose}`（resolve_agent 文件 fallback、effective_default 四级默认链、skill 多根遮蔽），写路径/NFS 只读导出（nfsserve 0.11，真实 mount 验证）在本 crate；session `/agent` 切换 + bash PATH 脚本前缀注入，web `/api/agents*` + SPA「Agent 配置」。

## 关键抽象

- `Store` trait（`crates/store/src/store.rs`）：sessions/messages/session_inputs/session_events/subagent_tasks 的统一 CRUD 口子，是切换 SQLite 实现的唯一接缝。
- `ChatStream` trait（`crates/llm/src/stream.rs`）：`ChatClient`（真）与 `MockChatClient`（测试）共同实现，使 session 运行时可零 token 确定性测试。
- 节点任务状态机（`crates/store/src/libsql_store/node_state.rs::transition_allowed`）：pending→running→done|error|cancelled，running/pending 经 cancelling 收束；终态冻结。失联收束：`Store::converge_lost_node_tasks(now_ms, stale_ms)` 单事务把「心跳超 stale 且状态 ∈ running/cancelling」的僵尸任务置 `error("node lost")`，由 `GET /api/nodes` 读路径机会式触发并对每个收束任务广播 `error` 终帧——worker 零代码变更即可重新领取。claim 靠 BEGIN IMMEDIATE 内条件 UPDATE CAS（不用 RETURNING），FIFO 以 `(created_at, rowid)` 定序——同毫秒 ULID 无单调性不可作 tiebreak。
- DAG run 状态机（`crates/dag/src/domain.rs` + store `dag.rs`）：pending→running→done|error|cancelled；claim 单活跃/节点 + FIFO `(created_at,rowid)` + BEGIN IMMEDIATE CAS；心跳 `cancel_run_ids` 捎带取消；失联收束把 running/cancelling 折叠 error("node lost")（复用任务面 converge 机制）。
- drain 语义（`crates/session/src/runner/mod.rs::run_loop`）：每个 turn 边界提升 steer；idle 边界逐条消费 queue，单 run FIFO 排空至 Done。doom-loop 守卫（`DOOM_THRESHOLD=20`，定义于 `runner/event.rs`）：滑动窗口内 20 个相同 `name:input` 工具签名 → Error + Err 终止 run。
- 请求签名协议（`crates/core/src/auth_sig.rs`）：canonical 四行串 + HMAC-SHA256(token) 小写 hex，头 `x-sig-timestamp`/`x-sig`；`crates/web/src/auth_sig_mw.rs` 负责 ±5min 窗口、进程内重放缓存（同签 409）、2MB body 上限（413），豁免 `/`、`/static/*`、`/api/time`、`/favicon.ico`。SPA 与 worker 节点共用此协议。

业务能力见 [features/index.md](features/index.md)。

## 仓库规则

开发必须遵循 [rules/](rules/) 目录下的规则：

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有对应测试用例
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 每轮迭代结束前全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层规范（unit / integration / e2e）
