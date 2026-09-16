Commit: (working-tree, ops-only)

# streaming-inspector 巡检 agent 注册与全链路冒烟通过（无代码改动）

为 viking-streaming 巡检能力（jy-hub / viking-streaming-observability 集群只读探测）注册平台自定义 agent `streaming-inspector`，经平台执行体完成端到端冒烟：prompt 组合、skills 发现、tools PATH 快照执行、NFS 分发全部生效。巡检能力本体仍在 `/data00/viking-streaming` 开发中，本条只落注册与验证。

## 注册内容（server 18081，agents root `/var/lib/opencoder-server/.opencoder/agents/`）

- 共享池 `prompts/streaming-inspect/v1`：soul/how/output 三段式（soul 定位只读巡检、fail-closed、不碰运行时；how 给 namespace 语义与证据链；output 固定 JSON 合同）。
- 共享池 `skills/streaming-inspect/v1`：`streaming-ops`、`inspect-cluster` 两个 SKILL.md（巡检配方与事实源指向 viking-streaming 仓库）。
- tools 池：`tools/streaming-inspect/v1` 创建后经编辑器流（`PUT /api/agents/{name}/resources/tools`）fork 为 owned `tools/agent-44c8d84cbfef43d7a31c23042aa1381e/v2`，脚本执行位 0755 生效；含 `vstream-ns-ops`（namespace 列举）、`vstream-flink-health`（固定合同单行 JSON）、`vstream-cluster-facts` 与 README。
- agent 卡 `streaming-inspector`（`meta.json`）：prompt/skills/tools 三引用 + memory null；history 记录 tools 从 `streaming-inspect` 切到 owned fork。

## 注册路径关键事实（复用必读）

- legacy create（`POST /api/agents/resources/{cat}`）硬编码 0600（`crates/agents/src/io.rs` `atomic_write`），mode 参数被忽略；编辑器流（SaveRequest）走 `write_files` 的 `set_permissions` 保留执行位，且首次编辑把共享池 fork 成 agent-owned 资源、卡引用自动切换——这是让 tools 0755 生效的必经路径。
- skills 池文件路径必须相对版本目录（`streaming-ops/SKILL.md`）；带 `skills/` 前缀报 400 `skill package 'skills' requires its SKILL.md entry`。
- NFS 只读挂载（`/mnt/opencoder-agents`，vers=3 port=2049）上直接 exec 0755 脚本报 `Permission denied`（exit 126，平台级）；执行时会话把池复制到本地快照并保留权限位（`crates/session/src/harness/resources.rs` `copy_tree`），`tools_path` 指向本地副本，故平台执行体可正常 exec。
- 凭据不进注册池（池会被 NFS 只读导出）；viking token 由包装脚本运行时 source `~/.config/viking-auth/viking-workspace-{prod,dev,dev-defaults}.env`（0600，仓库外）。核心 Flink/Prometheus 探测走集群内入口，无需 token。

## 冒烟验证

| 门 | 结果 |
| --- | --- |
| NFS 分发 | `/mnt/opencoder-agents/streaming-inspector/meta.json` 存在，三引用解析正确 |
| 平台执行体 | `agent-streaming-smoke-01`（kind=agent，target=streaming-inspector，node human-os-02）拉起成功；create 首包 504 用同 id 幂等重试即接管 |
| prompt 组合 | 模型按 soul/how 约束执行，reasoning 中直接引用 skill 合同原文 |
| tools 执行 | 会话内 `vstream-flink-health` 走 PATH 快照返回完整 JSON；`vstream-ns-ops --list` 返回 jy-hub、viking-streaming-prod |
| 输出 | 最终回答「异常」= prod 集群 `status=abnormal`（`parity_age_seconds` 1,124,101s 超门禁 93,600 + 6 条 critical 告警；Flink 拓扑本身 2586/2586 subtasks、lag 0.35s，属既有 prod 状态，非本次引入） |
| token 注入 | prod/dev 双环境 source 后注入成功（仅在会话内验证，未入池） |

## 遗留

- 巡检能力本体（探测项、门禁阈值、对账口径）以 `/data00/viking-streaming` 仓库 `agent-ops/`、`skills/inspect-streaming-cluster/` 为准，后续演进需同步池内 tools/skills。
- `vstream-flink-health` 的 `abnormal` 两项为 prod 集群既有状态，待流式侧 parity 对账补齐后复测。

## 相关

- `features/changelog/2026-09-17/dependency-analysis-agent-live.md` — agents 池注册与 NFS pin 先例。
- `crates/agents/src/io.rs`（0600 根因）、`crates/web/src/api_agent_resources.rs` 与 `crates/web/src/api_agents/resources.rs`（两条写路径）、`crates/session/src/harness/resources.rs`（执行时快照）。
