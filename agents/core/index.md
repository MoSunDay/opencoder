Commit: 6b8410c1bb5a175cfd8d0d6c2259fa357e136024

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 定义于相邻 crate，Config 供全仓加载。

## 索引
- [build.rs](../../crates/core/build.rs) 捕获构建提交和干净状态。正式发布通过 `OPENCODER_BUILD_COMMIT` 提供预期提交，作为跨工作树共享 Cargo 缓存的显式刷新条件；预期与实际检出提交不符时构建失败，不能产出混合版本包。
- [identity/permissions.rs](../../crates/core/src/identity/permissions.rs) — `admin`、`editor`、`viewer` 共用 HTTP 权限纯函数，Control、独立 Web 与 Ontology 使用同一读写判定。
- [harness/settings.rs](../../crates/core/src/harness/settings.rs) — Codex `startup_script` 保存逐项命令参数，配置与 profile 随执行固定；字段校验拒绝空入口和 NUL，Debug 不输出命令内容。
- `src/message.rs` — Message/Role/ContentBlock
- `src/config.rs` + `src/config/` — Config 加载、配置合并与基础配置；`src/config/runtime/` 保存 Agent、DAG、Ontology、MCP、CLI、技能等运行配置。物理目录拆分不改变 `crate::config` 的公共类型和序列化字段；顶层 `local_memory` 默认关闭，供会话完成钩子读取。`load_with_home` 将候选链重定向到执行 home；`load_with_home_frozen` 额外跳过 `apply_env`（快照即最终，版本化 Operator resume 用）；`load_operator(dir)` 只读 Operator 配置平面目录（`config.json` + 域文件，不做 env 合并）；`effective_domain_value`/`domain_file_for` 供节点 bootstrap 携带域视图
- [配置覆盖](../../crates/core/tests/config_overlay/main.rs) — Codex 与三类可配置 NFS 按显式字段覆盖；未填写的启动、凭据、环境变量和导出字段保留。运行配置保留未指定的 profile，同名项替换完整版本。私有配置容量同时检查单文件和最终合并结果，错误不包含配置值；实现见 [merge.rs](../../crates/core/src/config/merge.rs) 与 [private_settings.rs](../../crates/core/src/config/runtime/agent/private_settings.rs)。
- [config/runtime/dag.rs](../../crates/core/src/config/runtime/dag.rs) — 原生 DAG 的镜像、二进制池、源工作区、节点数据根与独立只读 NFS 配置；严格拒绝未知字段，不提供执行模式切换。约定见 [规则 04](../../rules/04-dag-execution-contract.md)。
- [config/runtime/ontology.rs](../../crates/core/src/config/runtime/ontology.rs) — 可选正文根 `ontology.files_dir` 与独立 NFS 的启用、监听地址、端口；默认关闭、`127.0.0.1:2052`，导出始终只读。正文根默认由 Server workdir 对应的数据根计算，与 `--data-dir` 中的数据库分别管理。
- `src/harness/` — `Harness::{Opencoder,Codex}` 与私有运行态；`fresh_runtime` 统一前端新会话的执行器、env、model 选择，`matches_requested_env` 校验续会话显式 env，托管配置可补充其他变量
- [harness/settings.rs](../../crates/core/src/harness/settings.rs) — Codex `startup_script` 保存外部启动命令数组，随私有 Harness 设置固定；校验入口和参数，不解释账号策略。接入约定见 [启动脚本](../../docs/codex-startup-script.md)。
- [harness/remote.rs](../../crates/core/src/harness/remote.rs) — `ServerConnection` 仅含默认关闭的 `enabled` 与 `url`；`ServerCapability` 表示能力 ID、种类、目标和摘要，`RemoteSession` 保存执行绑定与可重试首轮输入。已有 Harness JSON 保存 `remote` 与 `literal_mentions`，不新增表结构。
- `src/agent/`、`src/skill.rs` — Agent 引用卡（资源引用与 Harness，不包含宿主机/容器开关）、memory 池聚合（`agent/memory.rs`）与技能发现。技能根优先级：执行任务本地根（`skill::with_execution`/`execution_root`）→ 节点 pinned 根 → 真实 `~/.opencoder/skills`
- `src/skill/seed.rs` — 二进制内置 skill 增量 seed
- 仓库记忆 — 默认索引为 `repo-memory.md`；沿用仓库明确指定且与 `AGENTS.md` 大小写折叠后不同的索引，保持仓库指令文件不变。种子文件与实际文件分离约束见 [seeding.rs](../../crates/core/tests/skill_contract/seeding.rs)。
- [技能契约测试](../../crates/core/tests/skill_contract/main.rs) — 按发现、种子写入、规划与工作流分模块；首次安装核对准确的技能和资源集合，升级备份内置文件的用户修改，清单之外的用户资源保持原样。
- `src/tool.rs` — Tool trait / ToolContext / ToolOutput
- [platform/](../../crates/core/src/platform/mod.rs) — 执行种类、宿主命令语言、私有文件与原子发布的跨平台入口；Windows ACL 在创建文件时生效，路径校验拒绝设备名与重解析点，目录身份用于去重。
- [net.rs](../../crates/core/src/net.rs) — HTTP 客户端统一解析显式代理及大小写环境变量；读取 `NO_PROXY`（未设置时读取 `no_proxy`），并保留固定的回环地址例外。容器沿用配置中的代理，额外传入这两项排除变量；纯规则构造入口供测试与调用方避免读进程环境。
- `src/data_dir.rs` — 按工作目录隔离的数据目录
- `src/fleet/protocol.rs` — Server/Node 协议（PROTOCOL_VERSION = 10）
- [fleet/release.rs](../../crates/core/src/fleet/release.rs) — 发布交接协议为 1，存储 schema 为 v39；reader 过渡版留在数据格式 9，IO writer 用格式 10 标记新持久化绑定。当前发布工具只在读取能力匹配时允许 9/10 重叠，旧脚本也会因格式不同拒绝危险回滚。格式 1–8 仍须维护升级。
- [brain/io.rs](../../crates/core/src/brain/io.rs) — 缺少 IO 标记的请求固定为旧协议；显式版本必须是已支持的整数。[version.rs](../../crates/core/src/version.rs) 将 reader/writer 能力编入七个二进制，发布包据此校验启用与回滚顺序。
- `src/brain/` — 保存计划版本、能力描述与产物引用；`layered/` 是唯一分层计划及运行协议，schema 7 的每个节点绑定一个能力。`BrainInputBinding::Execution` 可选 `source: output | artifacts`，历史绑定默认 output；`ArtifactRef` 保留执行、步骤、可选实例 index、文件名、大小与 SHA-256。`execution_io_v1` 标识完整输入和文件索引传递能力。调度见 [brain](../brain/index.md)，执行面见 [worker](../worker/index.md)。

## 私有任务文件

`src/fleet/private_files/` 定义 `PrivateExecutionContext`、期限/路径/容量纯校验及不可变存储。Unix 执行目录 0700、文件 0600；Windows 使用受保护的 ACL，仅允许当前用户、SYSTEM 和 Administrators。拒绝链接，写入同步并校验回读；Debug 脱敏。`image_digest` 指运行中节点可执行文件 SHA256。`Config.dag.execution_private_root` 仅运行态传递，不参与配置序列化。
## 平台能力描述

[ontology](../../crates/core/src/ontology/mod.rs) 定义公开能力描述与平台入口清单，供控制面同步及 CLI 完整性检查共用。描述保存稳定来源标识、公开约定、入口和明确依赖；原始执行定义及凭据继续归属原模块。新增入口遵循 [能力同步规则](../../rules/09-ontology-capabilities.md)。

[fleet/report.rs](../../crates/core/src/fleet/report.rs) 定义可选增量报告：双方协商 ACK 与 delta 头后，Begin 携带 `mode/base_sequence`，Batch 可携带 `removed_ids`。旧全量 JSON 不变，协议版本仍为 10；基准使用已提交的 Snapshot sequence，不使用分片确认序号。

兼容边界：本次仅受管 Host 请求增量协商。旧 Server 对直连 Worker 不保存报告提交序号，无法防止迟到旧报告覆盖状态，因此直连 Worker 继续周期性全量；候选提取优化仍生效。Server 拒绝非 Host 代次启用增量。

[fleet/inventory/](../../crates/core/src/fleet/inventory/mod.rs) 保存有界的索引变更历史与固定分页快照。源游标过期后返回完整摘要，分页期间的变化留到下一次增量。读取和写入兼容由 [fleet/release/report_state.rs](../../crates/core/src/fleet/release/report_state.rs) 声明，并进入构建信息及发布清单。独立 ACK writer 使用数据格式 12，让旧控制器拒绝不兼容的发布和回滚；格式 11 的 reader 过渡包由新控制器验证后可以共存。`ontology_catalog_schema` 声明候选所需目录结构，缺省为 0。
