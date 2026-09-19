Commit: 0662b92479d7d82df82a604491c9ca15876bf2ad

# 信号发布 rel-0662b924（code-review 门禁 DAG M0–M2 上线）

## 发布路径

- 2026-09-18 信号发布上线 `rel-0662b924`（前序 `rel-34f69db1`）。`bash scripts/platform/deploy.sh --signal --bundle /srv/releases/rel-0662b924 --wait-seconds 300` phase=complete，target=`rel-0662b92479d7d82df82a604491c9ca15876bf2ad`；`.opencoder-platform-current` 已切换，新 server pid 471321:3081、runtime/host unit 均 `rel-0662b924`。
- 本批差异共 3 提交：`f5907145`（M0 知识库只读挂载 + M1 agent 步 runc 化 + M2 门禁 DAG 9 步 spec + 外来在途 WIP 快照，89 文件 +6517/−283）→ `61e760cf`（clippy：dag-review-tools manual_filter/manual_range_contains）→ `0662b924`（clippy：code_review.rs 无参 format! 改 to_string）。SPA 自上版零漂移（spa_sha256 `d3db9d05f4d3…` 一致）。
- 主 repo 被并行会话持续写脏（外来 cargo 抢 target 锁 + 混用 toolchain），沿用 worktree 惯例：`git worktree add --detach /tmp/oc-release-0662b924` + 独立 `CARGO_TARGET_DIR=/var/tmp/oc-release-target-0662b924`；门禁与 bundle 均在隔离环境产出。
- bundle `/srv/releases/rel-0662b924`：SHA256SUMS 全 OK、version_long `0.1.0 (0662b924)` 非 dirty；激活前备份 `/var/backups/opencoder/rel-61e760cf-preactivate-20260918`（972M，已从 repo 目录迁出）；旧 `rel-61e760cf` 已清理。

## 回归门证据（发布点 0662b924 全量，gate3 隔离）

- CLIPPY_OK（`-D warnings` 零警告）、BUILD_OK（bins 零错误）、`cargo test --workspace -- --test-threads=4 --skip review_dags::` 98 套件全绿；日志 `/var/tmp/opencoder-release-20260918/f5907145/{gate3.log,session-retest.log}`。
- 豁免二项（均有证据）：
  - `review_dags` 2 例：外来在途 WIP（其会话正重构 `tests/dag_e2e/review_dags` 与 `dag.ops` 注册表线），非本批引入。
  - session lib `tools::bash::tests::background_output_overflow_stops_process_and_caps_file`：与上版代码零改动，单线程 3/3 过 + 4 线程复跑 456/456 exit=0，定性高负载并发时序偶发。

## launcher 事故处置 ×2（与 09-17/09-18 前序同型）

- `/usr/local/bin/opencoder` 再次被写入裸 ELF（`0.1.0 (34f69db1-dirty)`）→ 备份 `opencoder.bak.manual-34f69db1dirty-20260918-1053` 后恢复 symlink。
- `opencode-cli` 历史坏链（指向不存在的 `bin/opencode-cli`）→ 部署后 installer 已自建正确链接。
- 上线复核：四 launcher 全为指向 `.opencoder-platform-current/bin/*` 的 symlink，版本 `0.1.0 (0662b924)`。

## 产线生效验证

- `/api/health` → `{"commit":"0.1.0 (0662b924)","ok":true,"protocol_version":10,"role":"control"}`。
- **`opencode-cli dag defs put --json @examples/dag/code-review.json` 入库成功**：9 步门禁 spec（kb-index → 范围 → API/护网双锚定 → 双路审查 → 重核 → 裁决 → viking 工单）上生产。
- probe-2（指定 node_id=node-01M1WVDEYE7Q4TFV6J83EZGKYJ）：agent 步全链 done（M1 runc 沙箱通道产线可用）。
- probe-3（compat dispatch 带 `--input` 透传）：done——M2 输入通道产线证据。
- 四节点心跳实时（1–4s）：human-os-02 / jyhub-macos-builder / eval-diagnose-node / windows-operator-node。

## standby host Connection refused 定性与修复（本批收尾排查）

- 现象：host（pid 471201）自 12:31 起每 2s 刷 `node channel disconnected ... Connection refused (os error 111)`，server 侧同步每 2s `node channel failed error=node already …`。
- 排查链：nginx 18081 WS 代理正常 → 手动带 Bearer 握手 18081/3081 均 101 → ss 显示 host 对 3081 主通道 ESTAB → `host.db fleet_definitions` 查明 **enabled 的 release_server 定义有两个**：当前 `rel-0662b924 → 3081`（正常）与遗留 `rel-2868ebfd → http://127.0.0.1:3039`（早已下线的历史 release 端口）。
- 根因：host 对每个 enabled 定义各跑一路 `opencoder_node::fleet::run` 循环（`crates/agent/src/host/mod.rs`）；3039 死端口每 2s refused，双路循环对 3081 产生重复注册竞态（server 侧 "node already"）。
- 修复：host 本地 API `POST :3083/servers/rel-2868ebfd…`（Bearer）置 `enabled=false`；30s 双侧零错误，主通道/心跳/执行面不受影响。
- 教训：deploy 下线旧 release 时应同步禁用其 release_server 定义（或在 host 端对 unreachable url 做衰减/熔断），否则遗留 enabled 定义会制造永久 2s 噪声循环。

## 已知事项

- probe-1 失败定性为 windows-operator-node 认领 Linux shell 探针（ENOENT os error 2，8ms 即败），与发布无关；跨平台探针需带 OS 亲和调度（后续项）。
- M3（真实节点 config `knowledge_root` / `agent_sandbox=runc` 激活、kb-index wasm 播撒、viking 工单 env 注入）与 M4（上线接入）属节点运维，待后续迭代。
- 遗留备份：`/usr/local/bin/opencoder.bak.manual-34f69db1dirty-20260918-1053`（dirty 裸二进制）待并行会话确认后处置。
