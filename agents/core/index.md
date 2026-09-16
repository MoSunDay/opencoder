Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件
- `src/harness/` — Harness::{Opencode,Codex} 与 Codex 运行态
- `src/agent/`、`src/skill.rs` — agent 引用卡/资源根与技能发现；memory 池目录化：读侧 `src/agent/memory.rs` 递归聚合版本目录全部 `*.md`（相对路径字典序、`\n\n` 分隔、200KiB 截断标记，对齐 `session/src/prompt.rs` 的 AGENTS_MD_MAX_BYTES）
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION）
- `src/brain/` — `schema_version: 2` 的 inputs/instances/outputs/routes、不可变计划版本、因果 visit/output 引用、局部路由回执与分别有依据的完成／验证判断。

## 接缝
- Config 单一真源；`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate。
- Fleet 协议为 10；Brain 新运行显式携带 v2，旧协议节点不能接收。领域推进在 [brain](../brain/index.md)，存储与派发在 [worker](../worker/index.md)。
