Commit: 23f41d38

# eval-diagnose 链路分阶段 DAG 化：专职节点 + 2.1 归因 / 2.2 建单注册与真实验证

把「skill 发现问题 → viking 工单」链路迁到 opencoder DAG 的第一阶段落地：专职节点固定 `/root/workspace`、生产服务器注册分阶段定义 `eval-diagnose-v2`（2.1 诊断/归因 → 2.2 幂等建单/追加），并以真实 viking prod 工单完成端到端验证。本次为线上定义与部署配置变更，未改动仓库源码。

## 部署变更

- 专职 agent `eval-diagnose-node`（`node-01M2PGY0C06Q55ZF7DV60WBA02`）：`systemd-run` 排障后固化为 `/etc/systemd/system/eval-diagnose-node.service`（`enable --now`、`Restart=on-failure`），启动参数 `--workdir /root/workspace --max-runs 2`，`EnvironmentFile=/etc/eval-diagnose-node.env`（0600，`VIKING_AUTH_TOKEN`/`VIKING_AUTH_PROD_ISSUED_TOKEN`，取自 `/root/.config/eval-diagnose-api/viking-token`）。scheduling 回读 `{"max_runs":2,"queue_order":"fifo","workdir":null}`——workdir 语义走启动参数（`crates/worker/src/brain/workdir.rs`），无需 `configure_scheduling`（multi-runtime host 上该 RPC 对 workdir 直接 400）。
- 生产 server（rel-653c7162，`--port 3045`，nginx 18081 反代）注册 `eval-diagnose-v2`：`diagnose`（agent=eval-diagnose，timeout 3600s，输出结构化结论 JSON fence）→ `viking-ticket`（depends_on diagnose，timeout 900s，幂等建单/追加）。现役单步 `eval-diagnose` 定义保持不变，bridge（`POST /api/executions` target 切换）为后续阶段。

## 运行时事实（本次实测）

- dispatch RPC 语义：`POST /api/dag/defs/:id/dispatch` 与 `POST /api/executions`（compat 层）都是 `hub.call` 15s 等节点 ACK；节点侧 `create()` 先做 agent 池 NFS pin（`crates/worker/src/operations/create.rs` → `resources::pin`）再回 ACK，常规耗时 ~13-15s，会撞上 15s 超时返回 `504 "node request timed out; retry using the same execution id"`——但任务照常入队并被节点执行。**生产配方：固定 id + 504 后同 id 重试 → 即得 202 `{run_id,execution}`；不可换 id 重试。**
- agent 步会话工作目录 = 节点启动 `--workdir`：探针步真实执行 `pwd` 输出 `/root/workspace`（工件 `<node-data>/dag/<run_id>/<step>/output.json`）。
- shellguard 不拦建单命令形态（P0-4 解除）：`viking-cli event session get`、写 `/tmp`、写 `/root/.local/share/eval-diagnose-api/events/**` 均放行，block_messages 为空。
- viking 命令合同在 DAG 会话内实测：`ticket create`（`--type-version-id` 建单时实时取、幂等键 `eval-diagnose:<job_id>:ticket:v1`、`--yes --viking-env prod --base-url … --output json`）、receipt 先落盘再 readback、`ticket export` 为读操作**不接受 `--yes`**、`comment create` 需 `--ticket-id`+幂等键。真实工单 `9c1b0caf-0e6d-47d5-a087-6eaa7f48572b` + comment `ac05d1f0-…` 为验收证据（标题带 [TEST][DAG迁移] 标记）。
- 分阶段 e2e `dag-edv2-e2e-0001`：`input.prompt` 注入 → 2.1 按 fence 输出 `{job_id(与 sha256(eventId) 派生一致), event_id, case_id, conclusions:[], gaps[], health:"degraded"}`（证据不足停机路径）→ 2.2 消费上游输出走 `not_required` 分支零副作用。上游 JSON 经 `depends_on` context 注入 + agent 步最后 ```json fence 恢复（`crates/dag-runtime/src/exec/agent.rs`）。

## 测试清单

| 门 | 结果 |
| --- | --- |
| opencoder `dag_e2e` 全套件（5 用例） | 5 passed / 0 failed（HEAD f43fb140 复跑）——`flow::dag_spec_dispatch_runs_wasm_and_agent_steps_to_done` 曾在 23f41d38 出现 3 连红（`run_doc["name"]` Null），归因为 deps 中 CLI 二进制瞬态中间状态，同 target dir 复跑与全新编译 CLI 均稳定 PASS，非未提交 `tests/support/llm_stub.rs` 所致 |
| opencoder `operator_e2e` | 2 passed / 3 failed——`flow`/`lifecycle`/`relay_sse` 断言 `POST /api/sessions` 需 `agent-` 前缀 id；测试夹具用 `operator-` 前缀。为并行迭代遗留失配（HEAD 已前进至 23f41d38，`tests/support/llm_stub.rs` 有他人未提交修改），与本次零代码变更无关，未越界修复 |
| eval-diagnose-cli `npm test` | 188 pass / 0 fail |
| viking 建单/追加真实链路 | 见上「运行时事实」，工单+comment 均 prod 实建 |

## 遗留

- `eval-diagnose-v2` 与 bridge 对接（`input.prompt` 注入、target 切换）待下一阶段；现役 `eval-diagnose` 定义未动。
- `eval-diagnose-shellguard-probe`/`eval-diagnose-viking-probe`/`eval-diagnose-workdir-probe` 三个探针定义保留在生产 defs 列表（验收证据），可在下一阶段清理。
- 专职节点经 `systemd-run` 排障期间发现：本机 bash 工具会回收后台子进程（setsid 亦不豁免），长任务一律走 systemd-run。

## 相关

- `features/changelog/2026-09-16/dag-agent-adaptation.md` — 存量 DAG 迁移到 agent 执行契约（单步形态与 prompt 合同来源）。
- `crates/worker/src/operations/create.rs`、`crates/control/src/api/executions/submit.rs`、`crates/control/src/transport/hub.rs` — dispatch/pin/ACK 语义出处。
