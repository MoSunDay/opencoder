Commit: be76fc1086cbf0d928c1d1e03ad5470563fd86df

# OpenCoder 逻辑地图

OpenCoder 是完全独立、从零实现的 Rust 原生编码代理。`opencoder` 提供本地 CLI/TUI，`opencoder-cli` 是远程管理客户端；`opencoder-server` 承担控制面，`opencoder-agent` 承担节点执行。workspace 由多个 crate 组成。所有上层依赖 `Arc<dyn Store>` / `Arc<dyn ChatStream>` 两个抽象口子，存储与 LLM 后端均可替换。

## 模块索引

- [agents/control](agents/control/index.md) — 平台控制面：全局定义、节点调度、执行索引和按 ID 查询节点明细。
- [agents/worker](agents/worker/index.md) — 节点执行面：本地接受/恢复、资源快照、agent/team/workflow/project 适配与维护工具。

- [agents/shellguard](agents/shellguard/index.md) — sandbox 模式 shell 命令安全分类器（rippy MIT 衍生）：rable AST 级判定，释放集仅 `/tmp` + `/dev/null`，cwd 不释放，不可解析 fail-closed；被 session 的 bash_guard 薄适配消费。
- [agents/store](agents/store/index.md) — 持久化抽象层。`Store` trait + libsql 实现（WAL，本地嵌入）。所有 session/message/input/event/subagent 与 TODO 工作流持久化的唯一出口。未来可切其它 Rust SQLite 实现。
- [agents/llm](agents/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` trait + `MockChatClient` + token 估算器。
- [agents/session](agents/session/index.md) — 会话运行时核心：drain 主循环（steer/queue 提升）、工具注册（内建 + MCP + latent：ssh_pty/question 按 skill 解锁）、subagent 调度（explore/build + libsql 追踪）、plan 只读 bash 写拦截（bash_guard → shellguard 分类核，cwd 对齐）、控制命令（/act、/plan、/act_clear_context）、压缩、resume、title 生成、cancel。
- [agents/core](agents/core/index.md) — 共享类型与 Config（模型/压缩/上下文窗口/small_model 全配置化）。
- [agents/web](agents/web/index.md) — axum HTTP + SSE 会话管理（prompt admit + 事件流 + 运行时切换 + interrupt）；纯 Bearer 认证中间件（`auth_mw.rs`，control 经 `#[path]` 复用；`/api/time` 与静态资源豁免）+ 编译期内嵌 React18+antd SPA（`spa/dist` 固定文件名 include_bytes! 白名单伺服）。
- [agents/local](agents/local/index.md) — `opencoder` 本地 CLI 前端：参数解析、headless、tmux 会话入口，以及 Harness 与环境配置转交。
- [agents/node](agents/node/index.md) — 出站 WebSocket 通信层：注册、心跳、即时负载/索引、执行 RPC 与系统维护 PeerBridge；保留旧 REST 接口供兼容。
- [agents/dag](agents/dag/index.md) — DAG workflow 纯域 + 线协议（spec/validate、ready_steps/run_outcome/render_context、`/workflow/<run_id>/<step>/` 工件契约、protocol DTO LOCKED）：server 校验与节点执行共享的唯一契约。
- [agents/dag-runtime](agents/dag-runtime/index.md) — 节点侧 DAG 调度与执行：Agent 会话、WebAssembly 步骤、取消和产物处理；仅 `opencoder-agent` 链接。
- [agents/server](agents/server/index.md) — `opencoder-server` 二进制：启动 control，不链接 session/team/project/DAG 执行引擎。
- [agents/agent](agents/agent/index.md) — `opencoder-agent` 二进制：构造 worker 并接入节点通道。
- [agents/ctl](agents/ctl/index.md) — `opencoder-cli` 远程管理客户端：封装 Server API、Bearer 认证、JSON/SSE 输出与退出码约定。
- [agents/tui](agents/tui/index.md) — ratatui 交互界面。
- [agents/todos](agents/todos/index.md) — 持久化 TODO 工作流运行时：父 Workflow Session 调度和验收，每个 TODO 使用独立 Primary Session 执行，支持依赖、并发、恢复、回退与可选 debug 投影。
- [agents/project](agents/project/index.md) — 用户策展的项目跟踪运行时（新 crate）：goal→milestone→todo 三级，todo 走「草稿→plan agent 生成方案→执行」生命周期，执行器可选 agent/team/dag/brain（大脑在平台预解析后下发），执行 resume 同一会话持续推进，`project_todo_runs` 版本留痕可取消；复用 session 直驱范式（非 todos 编排），项目数据走独立 `ProjectStore` 接缝（默认 libsql 同实例，feature-gate 可选 mysql/starrocks）。
- [agents/brain](agents/brain/index.md) — 能力库、向量检索和路由规划；平台由 control 承接能力绑定与直接派发。
- [agents/agents](agents/agents/index.md) — 版本化自定义 Agent（opencoder-agents crate）：共享池（prompts/skills/tools/memory/<名>/v{n}，版本只增、回滚=切指针）+ 引用卡（agents/<名>/meta.json 四字段引用，多 agent 共享同一资源）+ active marker；读路径在 core `agent::{meta,resource,compose}`（resolve_agent 文件 fallback、effective_default 四级默认链、skill 多根遮蔽），写路径/NFS 只读导出（nfsserve 0.11，真实 mount 验证）在本 crate；session `/agent` 切换 + bash PATH 脚本前缀注入，web `/api/agents*` + SPA「Agent 配置」。

业务能力见 [features/index.md](features/index.md)。

## 仓库规则

开发必须遵循 [rules/](rules/) 目录下的规则：

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有对应测试用例
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 每轮迭代结束前全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层规范（unit / integration / e2e）
