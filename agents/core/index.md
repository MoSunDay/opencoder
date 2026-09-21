Commit: 3b4775905c950f64433b5c9f4439f4396674b3c6

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate，Config 供全仓加载。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件（含 `config/dag.rs`）。`load_with_home` 将候选链重定向到执行 home；`load_with_home_frozen` 额外跳过 `apply_env`（快照即最终，版本化 Operator resume 用）；`load_operator(dir)` 只读 Operator 配置平面目录（`config.json` + 域文件，不做 env 合并）；`effective_domain_value`/`domain_file_for` 供节点 bootstrap 携带域视图
- `src/harness/` — Harness::{Opencode,Codex} 与 Codex 运行态
- `src/agent/`、`src/skill.rs` — agent 引用卡（`meta.json` `run_mode`）、memory 池聚合（`agent/memory.rs`）与技能发现。技能根优先级：执行任务本地根（`skill::with_execution`/`execution_root`）→ 节点 pinned 根 → 真实 `~/.opencoder/skills`
- `src/skill/seed.rs` — 二进制内置 skill 增量 seed
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION = 10）
- `src/brain/` — 保存计划版本、能力描述与产物引用；`layered/` 是唯一分层计划及运行协议。节点只有一句话任务、能力 ID 和重试策略。调度见 [brain](../brain/index.md)，执行面见 [worker](../worker/index.md)。
