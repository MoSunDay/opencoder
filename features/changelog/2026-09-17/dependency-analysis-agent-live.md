Commit: f2d723ed

# dependency-analysis agent 上线：强弱依赖分析 DAG 全链路真实跑通 + 节点 NFS pin EIO 根因修复

把旧 `viking-dependency-analysis-agent`（prompt 三段式 soul/how/output + 5 个分析 skill）迁到 opencoder agents 池体系，注册为 `dependency-analysis` agent 与单步 DAG 定义，并以真实输入（归档 33839）完成强弱依赖分析端到端验证（`depend_type=strong`，与旧系统验收样例一致）。期间定位并修复节点 pin 全量失败的 NFS 权限根因。后续复验发现首跑 `output.json=null` 的实证缺陷（结构化输出提取兼容性假设不成立），以 b765680c 修复并发版 rel-b765680c 复验通过（见「后续迭代」）。

## 部署变更

- 新 agent `dependency-analysis`（agents root `/var/lib/opencoder-server/.opencoder/agents/`）：池 `prompts/dependency-analysis/v1|v2`（soul/how/output 三段式，v2 为 soul.md 首行补描述段）、`skills/dependency-analysis-{link-locate,link-analyze,depend-judge,report-output,reference-facts}/v1`（旧 skill 名去掉 viking 前缀并修正 runtime expected 集合）、`tools/dependency-analysis/v1`；`meta.json` 引用卡（deterministic，期望 `dependency-analysis-runtime`）。旧名仅保留为工具名与 `.viking-dependencies/` 工作目录名。
- DAG 定义 `dependency-analysis`（server 18081）：单步 `analyze`（agent=dependency-analysis，timeout 3600s，spec prompt 携带唯一权威任务 JSON，仓库从 Code Explorer 归档接口下载并校验 repository_id/commit/SHA256），prompt 中调度方表述统一为 "the DAG scheduler"。
- 输出合同（report-output）：核心字段 `depend_type`（strong/strong_to_weak/weak/unknown，业务语义，非 DAG 调度语义）+ `analysis_report`（调用链路/函数签名与返回值分析/关键证据/判断理由/依赖类型）。~~当时假设"无围栏纯 JSON，`extract_output_json_from` 整段解析兜底兼容"~~——首跑实证该假设不成立（见下）。

## 根因与修复：节点 pin NFS EIO

- 现象：dispatch 全部 `504 "node request timed out"`，outbox 重试遇节点 409 `"execution directory exists without a durable journal record"`，节点侧只留空目录。
- 定位：`resources::pin`（`crates/worker/src/resources.rs`）把整个 agents root（150 卡/181 池/849 文件）复制为执行快照，任一文件不可读即整体失败。`/mnt/opencoder-agents/tools/viking-dep-continuous-r1/v1/__pycache__/archive_source.cpython-311.pyc` 读取报 `EIO (os error 5)`。
- 根因：该 `.pyc` 是 11:15 某 root 进程在池目录内跑 python 时生成的 `__pycache__`（`root:root 0600`），以 opencoder-server 用户运行的 `opencoder-resources.service`（NFS 导出）`File::open` 得 `EACCES`（`crates/agents/src/nfs.rs` `fs_error` → `NFS3ERR_IO/ACCES`），客户端表现为 EIO。服务重启/重挂载无效，因为错误在文件权限本身。
- 修复：`chown -R opencoder-server:opencoder-server` 该 `__pycache__` 后 NFS 读恢复、`resources::pin` 直连复现测试通过（10s 完成，无 EIO）。
- 504 语义确认：pin 全量复制在 NFS 上常规耗时 ~13-15s，会撞 15s RPC 超时；节点继续完成 pin 并写 journal，outbox 重试 create 幂等接管——与 `eval-diagnose-dag-staging` 记录的配方一致，504 后按同 execution id 重试即可。

## 真实验证（dag-1789618509-dep-eio-fix1）

- 输入：region=cn，`faceu.platform.api/GetPCShareDouyinVid` → `open.ability.aweme_video/EncryptPCVideo`，repository_id `7a45b9c3…`，commit `e6c39973…`（归档 33839）。
- 结果：run `done`（~4 分钟）；analyze 步 `outcome=done`，归档校验通过，三路定位收敛到唯一调用点 `biz/handler/get_pcshare_douyin_vid.go:39`（HTTP 注册 `biz/router/faceu/platform/api/platform_api.go:79`），产出最终 JSON `depend_type=strong`，`analysis_report` 五段齐全，与旧系统单真实验收样例（`single-real-33839-20260910`）判定一致。
- 残留：`execution_index` 中 `dag-01M2PPJ5DEPANALYZE0001`/`dag-01M2PP59HD0J5C04A174NC3MTW` 两条 pending 为孤儿索引行（run 记录已不存在，cancel 404），不影响新 dispatch；EIO 期间节点空目录 `dag-01M2PR8DEPANALYZE0004` 已清理。

## 后续迭代（b765680c）：结构化输出提取增强 + 发版复验

- 实证缺陷：首跑（dag-1789618509-dep-eio-fix1）`analyze/output.json=null`。根因：`crates/dag-runtime/src/exec/agent.rs` 的 `extract_output_json_from` 只认最后一个 ```json 围栏或整段纯 JSON；LLM 输出为 narration+JSON 混合时整段解析必败，run 虽 done 但无结构化产物。
- 代码修复（提交 b765680c）：三级提取 = 围栏 → 尾部裸 JSON 兜底（`extract_tail_bare_json`：8KB tail、string-aware 括号平衡、取最后一个可解析顶层对象；坏围栏时裸扫描从围栏体之后开始）→ 整段解析。同提交含 `copy_version` 失败路径上下文（T8）。
- 合同固化：prompts `dependency-analysis/v3`、skills `dependency-analysis/v2`（report-output 明确"末尾 ```json 围栏为规范形态 + 裸 JSON 兜底"）。
- 发版：干净 worktree build.sh → signal 滚动发版 rel-b765680c（server/runtime/host active，`/api/health` 返回 b765680c，protocol 10）。
- 真实验收（dag-1789631200-dep-structfix2）：同 33839 输入、同 spec，run done；`analyze/output.json` 非 null，`depend_type=strong`，`analysis_report` 五段齐全（依赖类型/关键证据/函数签名与返回值分析/判断理由/调用链路），含 `candidate_dependencies`/`static_chain` 等全字段。
- 调度注意：dispatch 未带 `node_id` 可能被调度到第三方在线 dag 节点（本次落到 jyhub-macos-builder，其池内无该 agent → `execution preflight: agent dependency-analysis unavailable`）；`POST /api/dag/defs/<def>/dispatch` body 支持 `node_id` 钉节点（本验收钉 runtime 节点 `node-01M1WVDEYE7Q4TFV6J83EZGKYJ`）。

## 测试清单

| 门 | 结果 |
| --- | --- |
| `resources::pin` 直连 NFS agents root（临时复现，已清理） | 修复前 `PIN FAILED: Input/output error (os error 5)`；修复后 ok（10.07s） |
| 本地 agents-root 副本 dag_e2e 复现 flow | create ~1.7s、dispatch→analyze done、output.json 产出（spec/池/prompt/JSON 提取全链路本身无问题） |
| opencoder `dag_e2e` 编译复验 | 清理 repro_tmp 后 `cargo test --test dag_e2e --no-run` 通过 |
| 线上真实 DAG run | `dag-1789618509-dep-eio-fix1` done，depend_type=strong（但 output.json=null → 引出本轮修复） |
| `dag-runtime` 提取单测（5 形态：围栏/裸 JSON/混合/多对象取末/字符串内花括号/8KB 边界） | 43 passed |
| `crates/worker` copy_version 错误路径上下文单测 | passed |
| `dag_e2e::structured_output`（两 agent 步：围栏形态 + 裸 JSON 尾形态，断言 output.json 非 null 且字段可达） | dag_e2e 5 passed |
| 线上复验 run（真实 33839） | `dag-1789631200-dep-structfix2` done，output.json 非 null，depend_type=strong |

## 遗留

- `execution_index` 两条孤儿 pending 行未清（操作 control.db 有风险，不影响新 run）。
- 池目录运行期生成 `__pycache__` 的隐患：文件名定位报错已由 b765680c（copy_version 上下文）落地；存量 root 属主已于同日巡检 chown 清零（见下节）。残余风险：任何再以 root 跑池内脚本都会重现权限污染，属运维纪律问题。
- `viking-dependency-analysis` 旧 agent 卡仍在线上 defs/池中并存，待旧链路下线后清理。

## 发布复核与属主巡检收尾（同日复验）

- 发布生效复核：`deploy.sh --status` current=rel-b765680c（manifest commit=b765680c，phase=complete，candidate=null），server/host/runtime 三单元 active；`/api/health` 返回 `0.1.0 (b765680c)` protocol 10。真实验收产物 `runtimes/rel-b765680c…/dag/dag-1789631200-dep-structfix2/analyze/output.json` 现场仍在：`depend_type=strong`、五段报告齐全。
- 聚焦回归复跑（干净 worktree b765680c）：`opencoder-dag-runtime` lib 43 passed；`dag_e2e::structured_output` 1 passed。
- 属主巡检（评审跟进①落地）：agents root 下 root 属主共 6 项清零——`tools/viking-dep-r3/v1/__pycache__`（含 0600 root 的 `viking_auth.cpython-311.pyc`，v1 回滚即 EIO 的实锤隐患）与 `memory/repo-chain-e2e-memory/` 整树，统一 chown 为 `opencoder-server:opencoder-server`；以 opencoder-server 用户实测 pyc/memory.md 可读。此后 v1 重新成为 current 不会复发 pin EIO。
- 调度抢单（评审跟进②，需另立迭代）：不带 `node_id` 的 dispatch 仍可能被无该 agent 的第三方在线节点接走，按 agent 卡可用性过滤候选节点待后续实现。

## 相关

- `features/changelog/2026-09-17/eval-diagnose-dag-staging.md` — dispatch/pin/504 配方与专职节点模式先例。
- `crates/worker/src/resources.rs`（pin/check_mount/copy_version）、`crates/dag-runtime/src/exec/agent.rs`（`extract_output_json_from`/`extract_tail_bare_json`）、`crates/agents/src/nfs.rs`（fs_error 映射）、`crates/control/src/transport/hub.rs`（15s 超时）。
