Commit: (working-tree, ops-only)

# jy-builder team operator 入队与四通道联调打通（零代码改动 · 纯平台注册 + 联调冒烟）

## 需求与结论
- 需求：把 operator 加入 jy-builder agent team，各打包环节同口径入队，完成 team 与各打包环节（mac 客户端 / windows 客户端 / lyra-cli / customagent）的联调打通。
- 结论：达成。team 扩为四成员，windows-build-verify capability 从内联形态（kind=operator/act）重绑到 operator 卡（kind=agent→operator），联调冒烟 turn1 participants=全四成员、四通道只读探针全绿。

## 注册面（全部经 opencoder-cli）
- prompts 池 `jybuilder-operator` v1（soul/how/output 三段式，与既有三成员同合同：恒 act + fail-closed + 制品必回读 + 凭证边界 + 单行 JSON 输出）。
- memory 池 `jybuilder-operator` v1（Windows 通道 + viking 上传通道 + 暂存区证据链知识）。
- agent 卡 `operator`：current 指针 prompt/skills(cpp-build-upload)/memory 全池全名，references 全绿（prompt_files=[soul,how,output]、skills=[artifact-upload,cpp-build]、memory=true）。
- capability `windows-build-verify`（brain-01M2QZ5SSJCEFZ79X6G3JDR2Y8）：原绑 `kind=operator,target=act`（内联形态）重绑 `kind=agent,target=operator`；回滚口径=重绑回 operator/act。
- team put：`jy-builder` members 扩为 [videofusion-win-operator(captain), lyra-cli-builder, customagent-builder, operator]。

## 联调冒烟（team-jybuilder-union-smoke-20260920b）
- 合同要点（新沉淀）：team 单 = kind=team + target=<team名> + node_id 钉定；缺 target→400 `target required`；**400 preflight 后同 id 不可复用（同 id 异输入→409），必须换新 id**（a→b 实证）。
- 创建遇 503 node disconnected（assignment retained）→ 同 id 幂等重试成功；期间 human-os-02 控制通道瞬态 offline→busy，节点恢复后自动拾起，未重派。
- 钉定 definition 实读：四成员 capabilities 各自冻结——videofusion-win-operator=macos-build、lyra-cli-builder=cpp-build(lyra)、customagent-builder=cpp-build(customagent)、**operator=windows-build-verify（win2 一键构建打包）**。
- 创建后 0 消息 ~8min：steer 注入尝试 400 未落（09-18 的注入先例本轮未复现成功，备用口径保留：`exec cmd --action steer --json @file`，JSON 合同 `{"prompt":"..."}`）；human-os-02 控制通道瞬态恢复后节点自动拾起 turn1，未重派（turn1 requirement 仍为原始需求单全文而非 steer 文本，实证未注入）。团队会话与 agent 会话不同：exec get 无 session 块，轮询判据用 result.final_summary + status=finished，勿用 agent 单的 output_text 判据。
- 结果：finished/complete，~1005s，turn1 participants=全四成员；四通道只读探针全绿——mac VM ssh probe_rc=0（vm=colima-k3s-arm，Linux aarch64 非 macOS，环境形态如实记录）/ lyra-cli /data00 HEAD=5edd661a6 rc-develop 干净 / customagent 检出 HEAD=dc73cf01 develop / operator win2 echo rc=0 + viking list `/jyhub/VideoFusion-win/release-11.6.0/0b1835ba…/` 5 制品可见（未打印 token）。
- fail-closed 行为留痕：operator 聚合侧曾对 customagent 通道误报 ok=false（候选路径缺前缀且无本轮证据），captain 只读复验 HEAD/branch 与成员自报逐字段一致后撤回改 ok=true；分歧以双方一致证据收口。
- 全程零构建/零上传/零写操作/零调度循环（只读冒烟）。

## 双通道口径（内联单 + team 路由并存）
- windows 环节现在两条等价路径：① operator pinned 内联单（windows-operator-node，无卡依赖，V-W1..V-W4 实证）；② team 路由派给 operator 成员（本轮联调实证探针可达，完整构建分片待首个 team 路由实跑单）。
- 内联形态回滚：`brain caps target-bind brain-01M2QZ5SSJCEFZ79X6G3JDR2Y8 --json '{"kind":"operator","target":"act"}'` 即恢复。

## 证据
- `/root/workspace/artifacts/test-agent/2026-09-20/jyhub-windows-build/team-jybuilder-union-smoke-20260920b-final.json`（含 turns/participants/members/final_summary 全文）
- 注册面回执：prompts/memory v1 回执、agents/operator meta、caps target-get、teams list 四成员
- workspace 文档：`agents/test-agent/build-team.md`（operator 入队 bullet + 四成员口径）、`index.md`、`dag-capabilities.md` 制品链路行

## team 路由 windows 真实构建首跑（P1，2026-09-20 补录）

- 单 `team-jybuilder-win-build-20260920a`（kind=team + target=jy-builder + 钉 human-os-02；创建 503 node disconnected 一次，同 id 幂等重试成功）。需求单=windows-build-verify input `{ref:0b1835ba…, mode:quick}`，prompt 带「构建分片只派 operator、不得代跑/扩围」硬约束。
- **验收判据全过**：turn1 构建分片 `participants=[operator]` 精准落位（sub_turns=1、aligned=true，未扩围代跑）；finished/complete；聚合回执 ok=true 含作业号 win-20260920tw1 / exit_code=0 / 产物 sha256 / source_lock / upload_readback=pass。
- 分片实绩：唯一构建作业 quick exit_code=0；win2 日志权威时长 06:21:06Z→06:27:57Z=**411s**（5 段 SKIP 同 ref 热跑），成员回执 seconds=691 为自派单起算口径，两口径并存以 win2 日志为准；`source-lock.json` locked_at=06:21:46Z 构建期刷新与 job 互证同 ref clean=true。
- 上传口径实跑验证：小件 pdb.7z(32B)+install_kit(5.6MB) 同名 409→`--overwrite` 重传一次成功，远端 revision 更新（0b8ed67b…/29a27dd0…），独立回读 sha256 与上传源及 V-W4 历史值逐字节一致（5bc3a063…/ddeb9542…）；大件未走 team 分片（内联单 V-W4 兜底）。全程 678.5s，含创建后 0 消息 ~8min 节点自愈观察窗（steer 备用未动用，turn1 requirement=原始需求单全文实证未注入）。
- **回滚演练**：caps 重绑 `agent→operator ↔ operator/act` 往返一次双向验证（target-get 逐向确认），终态复绑在线形态；期间无 create。
- 文档收口：workspace `agents/test-agent/build-team.md` 新增「上线 runbook」（四通道需求单模板/下单合同/判真判据/异常表/回滚三件/已知限制+capability 刷新口径），状态行改「已上线 + runbook 就绪」；captain/operator 记忆池沉淀本节事实（operator v2、captain v8）。证据：`/root/workspace/artifacts/test-agent/2026-09-20/jyhub-windows-build/{team-win-build-request.json,team-win-build-20260920a-final.json,win-20260920tw1-job.json}`。
