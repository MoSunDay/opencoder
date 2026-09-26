Commit: c60e2162be48102badf53d8b97e7cfa030b59605

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate，Config 供全仓加载。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件（含 `config/dag.rs`）；顶层 `local_memory` 默认关闭，供会话完成钩子读取。`load_with_home` 将候选链重定向到执行 home；`load_with_home_frozen` 额外跳过 `apply_env`（快照即最终，版本化 Operator resume 用）；`load_operator(dir)` 只读 Operator 配置平面目录（`config.json` + 域文件，不做 env 合并）；`effective_domain_value`/`domain_file_for` 供节点 bootstrap 携带域视图
- `src/harness/` — Harness::{Opencode,Codex} 与 Codex 运行态
- `src/agent/`、`src/skill.rs` — agent 引用卡（`meta.json` `run_mode`）、memory 池聚合（`agent/memory.rs`）与技能发现。技能根优先级：执行任务本地根（`skill::with_execution`/`execution_root`）→ 节点 pinned 根 → 真实 `~/.opencoder/skills`
- `src/skill/seed.rs` — 二进制内置 skill 增量 seed
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION = 10）
- `src/brain/` — 保存计划版本、能力描述与产物引用；`layered/` 是唯一分层计划及运行协议。节点只有一句话任务、能力 ID 和重试策略。调度见 [brain](../brain/index.md)，执行面见 [worker](../worker/index.md)。

## 私有任务文件

`src/fleet/private_files/` 定义 `PrivateExecutionContext`、期限/路径/容量纯校验及不可变存储。执行目录 0700、文件 0600，拒绝符号链接，写入同步并校验回读；Debug 脱敏。`image_digest` 指运行中节点可执行文件 SHA256。`Config.dag.execution_private_root` 仅运行态传递，不参与配置序列化。
