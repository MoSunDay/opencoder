Commit: (working-tree)

# rel-2868ebfd 生产行为验收回执（按 Agent 身份编辑配置）

2026-09-16 20:47–21:20 对已上线的 `rel-2868ebfd3fbd5f492d48cdc7c3282ff0df791263`（本机 10.199.71.70:3039，用户确认为 prod）做真实数据行为验收，补齐该发布「回归门豁免」后的验收缺口。验收期间旧 Runtime 进程身份不变、平台持续在线（`/api/ready` 全程 mode=open）。

## 验收结果（全部 PASS）

### 1. 资源四类型真实保存/读回（API 实质数据，非日志）

脚本：`/var/tmp/run_prod_acceptance.py`；证据：`/var/tmp/opencoder-prod-acceptance-20260916-204715/result.json`（14 步）。

- 空名新建 Agent（仅名称 + harness）→ 201，重名 409。
- Prompt 首存（soul/how/output）→ 读回字节一致；二改单文件后其余文件字节保留、版本递增 [1,2]；过期基线保存 → 409 `resource changed; reload before saving`；restore v1 生成 v3 且正文回到 v1，versions=[1,2,3]。
- Skills 首存含 `canary-skill/SKILL.md` + 256 字节全值域二进制附件（0o644），读回字节/权限一致。
- Tools 首存 0o755 可执行脚本，读回 mode=0o755。
- Memory 首存/读回一致。
- 不安全路径（`../escape`、`a//b`、`a/./b`、`a/../b`）真实基线下全部 400。
- 内置 `act`：GET 展示真实 builtin_prompt 且 `read_only=true`；PUT → 403 `builtin agent resources are read-only`。
- 注册卡列表含新卡；DELETE 卡片 200，被删卡的 `agent-<uuid>` 专属目录确认留存（NFS 导出可见、meta 标 owner_agent）——即已记录的孤儿资源已知行为。

### 2. 真实会话注入 + 运行时快照 pin

脚本：`/var/tmp/run_prod_injection.py`；会话 `operator-01M2N5Y7MTYP2815KH2GEC1C57`（消息归档 `/tmp/inj-msgs.json`）。

- Agent `prod-inject-89564374` 四类资源落盘后，真实发起 operator 会话（control → node-01M2WVDEYE7Q4TFV6J83EZGKYJ）。
- 节点受理快照目录 `/var/lib/opencoder-platform/runtimes/rel-2868ebfd.../operator/<sid>/resources/` 实测：prompt/skill（含 256 字节 blob）/0o755 tool/memory 四类 canary 字节全部 pin 入；无 `.staging~`。
- 经 `/api/sessions/:id/prompt` 中继发送 canary 复述指令，真实模型（harness_profile=eval-diagnose，gpt-6-astra）会话返回 assistant 正文恰为 `INJECT-1789564374`，证明 prompt 注入链路在线上对真实模型生效。（首次尝试未绑 profile，codex 走 OpenAI oauth 失败 `Reconnecting... auth.openai.com`，属 harness 配置问题，非本功能回归；绑定既有可用 profile 后通过。）

### 3. 跨 Agent 隔离与旧 API 身份门

脚本：`/var/tmp/run_prod_isolation.py`，16 步全 PASS。

- 共享池资源被 A/B 两卡引用；B 首存 fork 为新 `agent-<uuid>`（版本历史 [1,2] 完整、未改文件保留）；A 仍引用共享池原版本、字节不变；共享池本身不变。
- 旧共享池 API 写 B 的专属资源 → 403 `edit private resources through their agent`。
- A 绑定 B 的私有资源 → 400 `cannot bind another agent's private resource`。
- 删除被引用的共享资源 → 409 且响应给出 `referenced_by`；解绑后删除成功。

### 4. 15 分钟生产观察

脚本：`/var/tmp/run_prod_observe.py 900`；证据：`/var/tmp/release-observe-8e11a351d00a37ec/`（result.json + observation.json）。

- 161 个真实 WASI probe DAG 全部 done；最大受理间隔 0.167s、最大端到端 1.44s；全程 `ready=open/1 node`；观察期间每轮附带稳定 Agent 资源读回（eval-diagnose v2、3 文件）。

### 5. 回归门证据（发布提交的整批代码）

- `cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo test --workspace -- --test-threads=4` 403 二进制 / 5264 passed / 0 failed（独立 target dir）；`cargo build --workspace` 零错误。日志 `/var/tmp/opencoder-release-20260916/06-workspace-test.log`、`05-full-gate.log`。
- 专项：agent_resources 10、control resource_root 7、worker resource_snapshot 1、opencoder-agents 31（1 既有 ignored）、team topic_contract_flow 1、web team_topic_sequence 1、control e2e compat 三件、dag-runtime run_loop 8、cli server_local 全绿。
- SPA：104 files / 752 passed，vite build，`check-spa-drift.sh` 无漂移。发布工具链：signal 12 / rolling 19 / platform 19 / smooth_release 4。
- 在线备份：`deploy.sh --backup prod-observation-20260916`。

## 已知事项

- 删卡孤儿 `agent-<uuid>` 目录不回收（用户裁决本轮不做 GC），见 [agent-owned-resource-editor](agent-owned-resource-editor.md)。
- 工作树在发布后存在并行会话在途改动（全局 active marker 移除→会话级 agent、DAG run name 兼容层顶层化、host workdir_supported 能力位、brain 重构等），按用户裁决等待其落地后整批复验、整包再发；本回执只对已上线的 rel-2868ebfd 负责。
