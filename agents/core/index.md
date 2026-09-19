Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件；`config/dag.rs` 的 `knowledge_root`（知识库只读挂载根）与 `agent_sandbox`（host|runc，节点级 agent 步沙箱开关，DTO 不变）
- `src/harness/` — Harness::{Opencode,Codex} 与 Codex 运行态
- `src/agent/`、`src/skill.rs` — agent 引用卡/资源根与技能发现；memory 池目录化：读侧 `src/agent/memory.rs` 递归聚合版本目录全部 `*.md`（相对路径字典序、`\n\n` 分隔、200KiB 截断标记，对齐 `session/src/prompt.rs` 的 AGENTS_MD_MAX_BYTES）；不可读/非 UTF-8 文件降级跳过并输出 tracing::debug，写侧（web `safe_rel_path` / agents `validate_path`）同步拒绝点前缀隐藏段以对齐读侧跳过语义
- `src/skill/seed.rs` — 二进制内置 skill 的增量 seed：`task-plan` 闭环规划、`task-plan-subagent` 委派拆分、`say-and-replay` 只读对齐快照，以及执行/评审/提交和本地记忆配套；规划清单与委派清单作为同目录 references 一并投放，缺失文件补写、漂移文件备份后更新
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION）
- `src/brain/` — `schema_version: 2` 的 inputs/instances/outputs/routes、不可变计划版本、因果 visit/output 引用、局部路由回执与分别有依据的完成／验证判断。

## 接缝
- Config 单一真源；`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate。
- Fleet 协议为 10；Brain 新运行显式携带 v2，旧协议节点不能接收。领域推进在 [brain](../brain/index.md)，存储与派发在 [worker](../worker/index.md)。
