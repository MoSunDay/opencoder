Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate，Config 供全仓加载。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件（含 `config/dag.rs`；`load_with_home` 将候选链重定向到执行 home，Operator 隔离用）
- `src/harness/` — Harness::{Opencode,Codex} 与 Codex 运行态
- `src/agent/`、`src/skill.rs` — agent 引用卡（`meta.json` `run_mode`）、memory 池聚合（`agent/memory.rs`）与技能发现
- `src/skill/seed.rs` — 二进制内置 skill 增量 seed
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION = 10）
- `src/brain/` — brain 计划/图/输出/路由 DTO（推进调度见 [brain](../brain/index.md)，执行面在 [worker](../worker/index.md)）
