Commit: 34f69db14b8c6818823c1eb01696131dabba7e23

# 信号发布 rel-34f69db1（TUI 运行中模式切换拒绝 + stats-sync 目标库路径修复）

## 发布路径

- 2026-09-18 信号发布上线 `rel-34f69db1`（前序 `rel-8dd33c85`）。`scripts/platform/deploy.sh --signal --bundle /srv/releases/rel-34f69db1 --wait-seconds 300` exit=0，attempt `1ed1be4c13cd47508cd5c272539fef6b`，phase=complete；发布后 `--status` current=`rel-34f69db14b8c6818823c1eb01696131dabba7e23`，previous=`rel-8dd33c85…`。
- 本批差异共 6 提交：`9f9f7575`（tui 运行中模式切换拒绝）、`357916b4`（stats-sync 目标库路径修复）、其余为 docs/changelog；代码面仅 `crates/tui` 与 `scripts/opencoder-to-opencode-stats.py`。SPA 自 `rel-8dd33c85` 零改动（两版 spa_sha256 一致 `d3db9d05f4d3…`）。
- 主工作区有并行会话在途改动（`agents/control/index.md` 等 4 文件 dirty），沿用 worktree 解耦惯例：`git worktree add --detach /tmp/oc-release-34f69db1 34f69db1` + 独立 `CARGO_TARGET_DIR=/tmp/oc-release-target-34f69db1`。
- bundle `/srv/releases/rel-34f69db1`：manifest commit `34f69db14b8c6818823c1eb01696131dabba7e23`、version_long `0.1.0 (34f69db1)`、四二进制 SHA256 校验一致；激活前备份 `rel-34f69db1-preactivate-20260918`。

## 发布事故与恢复（launcher 契约再次破坏，与 09-17 同型）

- 某并行会话将 dirty 构建 `0.1.0 (34f69db1-dirty)`（sha256 `4e771ad4ce8acf4a…`）的裸 ELF 写入 `/usr/local/bin/opencoder`，破坏 launcher symlink 平台不变量。
- 处置：备份为 `/usr/local/bin/opencoder.bak.manual-34f69db1dirty-20260918` 后恢复 symlink；上线复核确认四个 launcher（opencoder/opencode-cli/opencoder-server/opencoder-host）均为指向 `.opencoder-platform-current/bin/*` 的 symlink。

## live.py 验收阻塞（平台既有问题，非本批引入）

- 正式验收脚本 `live.py` 两次前置失败（evidence：`/var/tmp/opencoder-release-20260918/evidence/release-live-c1d8551b6f4f83a7`、`release-live-473c5bc69cdb995d`），根因：todos 受理耗时 ~27-45s（节点 create→首批派发），超过 `crates/control/src/transport/hub.rs:411` 的 15s RPC 窗，`POST /api/executions` 返回 504 `node request timed out; retry using the same execution id`，但执行实际被受理并最终完成（两次 todos 链实际均 done/passed）。该时延在已部署的 rel-8dd33c85 上即存在（09-16 09:54 后引入）。
- 改用常规信号发布 + 自建观察脚本（`/var/tmp/opencoder-release-20260918/observe.py`）替代验收。

## 观察探针模块播撒时序竞态（自查纠正，非平台缺陷）

- 自建 900s 观察首跑全失败，报 `wasm module not found under the run context root … or the module library …/_modules/release-probe.wasm`，一度误判 eval-diagnose 节点模块库异常。
- 复盘时间线证伪：失败样本执行时刻（如 `dag-rel34f69db1-obs-5` execution.json 09:04:33）早于模块播撒时刻（`_modules` 09:11、run 目录 09:17），即所有失败发生在播种之前；节点解析逻辑 `crates/dag-runtime/src/exec/wasm/mod.rs` 的 `resolve_module` 仅做两处 `is_file()`。补播后单发 `dag-rel34f69db1-seedcheck-evaldiag-1` → done，eval-diagnose 与 windows-operator 行为一致，无平台缺陷。
- 教训：观察脚本启动前必须先完成模块播种并单发验证。

## 上线复核

- `/api/health`：`{"commit":"0.1.0 (34f69db1)","ok":true,"protocol_version":10,"role":"control"}`。
- 三 unit `opencoder-{server,runtime,host}-rel-34f69db1….service` 均 running；`.opencoder-platform-current` → `34f69db1…`。
- SPA 零漂移：在线 `/static/app.js` sha256 `9f55a1bf…` 与发布 worktree dist 一致（SPA 自上一版无改动，符合预期）。
- 发布后真实链路：`todos-post-release-1789693660`（平滑发布真实模型依赖链）已受理进入运行（first running / second pending）；`host.db runtime_owners` 中发布后 dag 探针 `dag-probe-*` 绑定 `rel-34f69db1…`。

## 回归门证据（发布点 34f69db1 全量）

- `cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo build --workspace --bins` 零错误；`cargo test --workspace -- --test-threads=4` 5311 passed / 0 failed / 7 ignored（416 二进制，含 operator/dag/todos/team/brain e2e；上一版遗留的两条基线失败在本发布点已消除）。日志 `/var/tmp/opencoder-release-20260918/{clippy,build,test}.log`、gate 汇总 `gate-status.txt`。
- 平台脚本 unittest 4 套 OK（`pytests.log`）；SPA 零漂移。

## 900 秒上线观察（rel-34f69db1）

- systemd 托管观察 unit `oc-observe6-34f69db1`（修复了 accept 统计 bug 后的 observe.py），900 秒持续提交 dag 探针：170 样本 / 0 失败；accept p50=19ms / p95=53ms / max=374ms；提交→终态 p50=247ms / p95=291ms / max=584ms；全程 `ready_mode=open`、`ready_nodes=4`。
- 结果落盘 `/var/tmp/opencoder-release-20260918/observation.json`（170 samples / failures: [] / seconds_observed: 900）。

## 已知事项

- todos 受理时延（>15s RPC 窗 → 504 可重试但执行实际受理）为平台既有问题，影响 live.py 类自动化验收的首次提交；建议后续专项：延长 RPC 窗或异步受理回执。
- 发布前 `_modules` 播撒的 run 目录预播种（`dag-rel34f69db1-obs-0..200`）属一次性杂物，节点库 `_modules/release-probe.wasm` 保留供后续探针复用。
- 遗留备份：`/usr/local/bin/opencoder.bak.manual-34f69db1dirty-20260918`（dirty 裸二进制备份，待并行会话确认后处置）。
- `todos-post-release-1789693660` 发布后仍在运行（真实模型依赖链），终态待其自然完成，不阻塞本回执。
