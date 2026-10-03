Commit: 40dd45ed4c7c3240a0e879ac5bfec391ffb5a03c

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate，Config 供全仓加载。

## 索引
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载与 mcp/cli/skills/ap 域文件（含 `config/dag.rs`）；顶层 `local_memory` 默认关闭，供会话完成钩子读取。`load_with_home` 将候选链重定向到执行 home；`load_with_home_frozen` 额外跳过 `apply_env`（快照即最终，版本化 Operator resume 用）；`load_operator(dir)` 只读 Operator 配置平面目录（`config.json` + 域文件，不做 env 合并）；`effective_domain_value`/`domain_file_for` 供节点 bootstrap 携带域视图
- [config/dag.rs](../../crates/core/src/config/dag.rs) — 原生 DAG 的镜像、二进制池、源工作区、节点数据根与独立只读 NFS 配置；严格拒绝未知字段，不提供执行模式切换。约定见 [规则 04](../../rules/04-dag-execution-contract.md)。
- `src/harness/` — `Harness::{Opencoder,Codex}` 与私有运行态；`fresh_runtime` 统一前端新会话的执行器、env、model 选择，`matches_requested_env` 校验续会话显式 env，托管配置可补充其他变量
- [harness/remote.rs](../../crates/core/src/harness/remote.rs) — `ServerConnection` 仅含默认关闭的 `enabled` 与 `url`；`ServerCapability` 表示能力 ID、种类、目标和摘要，`RemoteSession` 保存执行绑定与可重试首轮输入。已有 Harness JSON 保存 `remote` 与 `literal_mentions`，不新增表结构。
- `src/agent/`、`src/skill.rs` — agent 引用卡（`meta.json` `run_mode`）、memory 池聚合（`agent/memory.rs`）与技能发现。技能根优先级：执行任务本地根（`skill::with_execution`/`execution_root`）→ 节点 pinned 根 → 真实 `~/.opencoder/skills`
- `src/skill/seed.rs` — 二进制内置 skill 增量 seed
- [技能契约测试](../../crates/core/tests/skill_contract/main.rs) — 按发现、种子写入、规划与工作流分模块；首次安装核对准确的技能和资源集合，升级备份内置文件的用户修改，清单之外的用户资源保持原样。
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- [platform/](../../crates/core/src/platform/mod.rs) — 宿主命令语言、Windows 用户目录、私有文件与原子发布入口；Windows ACL 在创建文件时生效，路径校验拒绝设备名与重解析点。
- `src/net.rs`、`src/data_dir.rs` — HTTP 客户端与 per-workdir 数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION = 10）
- [fleet/release.rs](../../crates/core/src/fleet/release.rs) — 发布交接协议为 1，数据格式固定为 2；原生 DAG journal 与项目 schema v32 需要停服迁移，数据格式 1 不在兼容滚动发布范围。
- `src/brain/` — 保存计划版本、能力描述与产物引用；`layered/` 是唯一分层计划及运行协议。节点只有一句话任务、能力 ID 和重试策略。调度见 [brain](../brain/index.md)，执行面见 [worker](../worker/index.md)。

## 私有任务文件

`src/fleet/private_files/` 定义 `PrivateExecutionContext`、期限/路径/容量纯校验及不可变存储。Unix 执行目录 0700、文件 0600；Windows 使用受保护的 ACL，仅允许当前用户、SYSTEM 和 Administrators。拒绝链接，写入同步并校验回读；Debug 脱敏。`image_digest` 指运行中节点可执行文件 SHA256。`Config.dag.execution_private_root` 仅运行态传递，不参与配置序列化。
