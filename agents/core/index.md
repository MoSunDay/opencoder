Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件
- `src/harness/` — Harness::{Opencode,Codex} 与 Codex 运行态
- `src/agent/`、`src/skill.rs` — agent 引用卡/资源根与技能发现
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION）

## 接缝
- Config 单一真源；`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate。
