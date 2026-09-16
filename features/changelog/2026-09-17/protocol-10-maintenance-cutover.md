Commit: 82a30cf8（发布时基线）/ rel-a51016ca8878ff549460b47081c7a927813aeb64（已上线二进制）

# Fleet 协议 9→10 维护切换（大脑固定图 v2 上线）

2026-09-17 01:07–02:50（CST）对本机生产（10.199.71.70:18081，nginx→3039/3041）执行维护窗口，把 fleet 协议从 9 升级到 10，上线大脑固定图 v2（commit a51016ca 构建 rel-a51016ca...；含 01c14802 写侧隐藏路径收口，随后续发布）。新旧协议禁止滚动重叠，按既定维护迁移流程停旧起新。

## 切换前置

- **发布包**：`scripts/platform/release/build.sh --output /srv/releases/rel-a51016ca...`，manifest `protocol_version: 10`、编译 commit a51016ca 无 dirty、SPA digest 与 dist 树一致；四二进制 SHA256SUMS 校验通过。
- **全量门（a51016ca 干净树）**：`cargo clippy --workspace --all-targets -- -D warnings` 零告警；`cargo test --workspace` 406 二进制 5223 passed / 0 failed / 7 ignored；`cargo build --workspace` rc=0。SPA 109 文件 799 用例全绿、vite build 后 `check-spa-drift.sh` no drift。发布工具链 signal 12 / rolling 20 / platform 19 / smooth_release 4 全绿；todos 契约 e2e 44/44。
- **旧 brain 守卫数据处理（用户授权）**：9 个 2026-09-08/09 的 interrupted v1 brain agent 运行（带 brain_receipt）阻塞升级；在 `/var/lib/opencoder-node/agent/<id>/execution.json` 把状态归档为 cancelled 并写 `lifecycle.archive_reason`（审计时间 + 授权），独立归档目录 `/var/lib/opencoder-platform/backups/legacy-brain-v1-archive-20260916/`。worker journal 守卫复扫 4343 条 execution.json 零阻塞。
- **业务窗口对齐**：等待在途真实评审 DAG `dag-pingce-82ef...` 五步全部 done（00:22 受理，01:05 终态）后才冻结受理，未打断任何用户工作。
- **备份**：冻结后 `deploy.sh --backup pre-v10-20260917-0107`（host/runtimes/server 全量在线备份）；切换证据目录 `/var/tmp/opencoder-cutover-20260917-0106/`（health/ready/release-state 前置快照、host.db 各阶段备份）。

## 切换序列

1. `POST /api/admin/drain` 持久冻结；确认节点 active_runs=0、无 busy 执行。
2. 停 proto9 三件：`opencoder-runtime/server/host-rel-2868ebfd...`（独立 resources 服务保持运行）。
3. 克隆当前 runtime 数据到新一代目录 `runtimes/rel-a51016ca...`（继承全部 execution 历史、runtime.db、node-id），复用 3040/3039/3041 端口（nginx 无需改动）。
4. 生成新 systemd 三件（units.prepare/prepare_host/prepare_server 原语）并启动 runtime；runtime 首次启动继承冻结态，经 RPC `admission reopen` 后 ready。
5. host 跨代清理：新 host 校验「runtime endpoint owner」「execution runtime ownership」，从 host.db 删除 9 个 proto9 runtime 行、把 4572 条 runtime_owners 全部重新绑定到新 runtime 后，host 正常启动；`/runtimes/<id>/activate` + `/activate-host` 完成新一代激活。
6. server 启动后经 nginx 健康检查 protocol_version=10；`DELETE /api/admin/drain` 复开受理；公共 release-probe DAG 通过真实调度 done；journal 重写为仅含新一代（旧 proto9 journal 备份 `release-state.json.proto9-generation`），`install_bundle.py` 更新 `/usr/local/bin` 启动器。

## 发布后验证（真实数据）

| 项 | 结果 |
| --- | --- |
| `/api/health` | commit 0.1.0 (a51016ca)、protocol_version 10、role control |
| 在线 SPA | served app.js sha256 与 a51016ca commit dist 逐字节一致 |
| 资源四类型 CRUD | 14 步全 PASS（创建/首存/二改保留/409/restore v1→v3/危险路径 400/内置 act 只读 403/注册卡列表/删卡孤儿留存） |
| 跨 Agent 隔离 | fork 完整历史、旧池 API 写专属资源 403、绑他人私有资源 400、被引用资源 409 全 PASS |
| 真实模型注入 | 自定义 Agent 四类资源（含 256 字节 blob、0755 tool、多文件 memory：memory.md+topics/a.md+b.md）受理后 pin 入 runtime 快照，eval-diagnose profile 驱动 gpt-6-astra 原样返回 canary `INJECT-V10-*` |
| 会话级 Agent 切换 | `POST /api/sessions/<id>/agent {"value":"act"}` 200，inspect 投影 meta.agent=act；agents 列表不再有全局 active 字段 |
| 历史连续性 | pingce DAG 及全部旧 execution 经新 runtime 可读；节点身份 node-01M1WV... 不变；第二节点 jyhub-macos-builder 以协议 10 重连 |
| 平台观察 | 900 秒 161 个真实 release-probe 全部 done，最大受理 0.17s；期间真实业务 pingce DAG 持续在新栈正常跑完多轮 |

## 已知事项与回滚

- **回滚**：nginx 入口与端口不变，回滚为重装 rel-2868ebfd 三件并恢复 `release-state.json.proto9-generation` 与 host.db 备份；旧 bundle/unit 全部保留。
- **运行时注意**：节点对会话创建有短暂 `node request timed out/node offline`（控制面到 runtime 的稳定 ID 重试契约覆盖，同 ID 重试即成功）；未带 node_id 的自定义 Agent 首次请求可能超时，重试收敛。
- 协议 10 的进一步发布恢复信号平滑通道；proto9 journal 与 unit 文件保留在 `systemctl list-unit-files`（disabled），仅新一代 active。
- 证据：本目录、`/var/tmp/opencoder-prod-acceptance-20260917-013216/`、`/var/tmp/release-observe-5a761e213f539b17/`、各 backup 目录。

## 追发：HEAD 逻辑生效（rel-2dc1323d，同日 08:40）

维护切换上线的 a51016ca 落后 main 若干提交；按「仓库最新逻辑生效」要求，将 origin 同步后的 HEAD（`2dc1323d`，已推送 `940d8746..2dc1323d`）构建为 rel-2dc1323d 并经**信号平滑发布**（同协议 10，新旧可共存）上线，无停机：

- 相对 a51016ca 的生产代码差异仅三处（均为按 Agent 身份编辑资源的契约收口，commit `01c14802`）：`web/api_agent_resources.rs` safe_rel_path 拒绝点前缀隐藏段、`agents/resources/model.rs` validate_path 同步拒绝 `.x`/`x/.y`、`core/agent/memory.rs` section_body 对不可读/非 UTF-8 文件 debug 降级（读侧 collect 本就跳过隐藏文件，堵上「可写不可注入」缺口）。其余提交为 docs/test/ops。
- 发布前置：journal 文件属主修正为 opencoder-server:opencoder-server 0640（切换脚本 root 写入导致 `/api/admin/release` 500），恢复后 signal_protocol=1；HEAD 全量门 5224 passed / 0 failed、clippy 零告警（沿用当夜证据）。
- 发布：`rolling_cli --signal --bundle rel-2dc1323d --wait-seconds 300`，phase complete，旧 a51016ca 进入退役；nginx 上游切到 3042/3044，健康检查 commit `0.1.0 (2dc1323d)` protocol 10，两节点（human-os-02、jyhub-macos-builder）在线。
- 线上契约复验：隐藏段写入 `.hidden.md`/`dir/.dot.md`/`topics/.x.md` 全部 400「unsafe file path」；资源四类型 CRUD 14 步全 PASS；跨 Agent 隔离 16 步全 PASS；真实 gpt-6-astra 会话注入 canary `INJECT-HEAD-*` 模型原样回读；线上 app.js sha256 与 2dc1323d commit dist 字节一致（`04862d2e…`）。
