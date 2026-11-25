Commit: 0b2016c5a

# Fleet 本机发布 0b2016c5a 与 18080 workdir 自愈

## 背景

- 现象：`opencoder-server.service`（:18080）长期 `activating (auto-restart)`，
  `status=200/CHDIR`，累计重启 42,824 次；node02/node04 持续
  `Connection refused`。
- 根因：`WorkingDirectory=/tmp/oc-server` 指向易失目录；`/tmp/oc-*` 被外部
  清理后，systemd 在执行 `ExecStartPre`/`ExecStart` 前 chdir 即失败，
  `Restart=on-failure` 每 5 秒无限重试。首次失败 2026-11-16 21:17。
- 数据面：workdir 路径未变，数据目录
  `~/.local/share/opencoder/41f4b558559c2238/server-v2` 完整保留。

## 变更

- `/etc/systemd/system/opencoder-server.service`：`WorkingDirectory=/`（第 9 行）
  + `ExecStartPre=/usr/bin/mkdir -p /tmp/oc-server`（第 12 行），保留
  `--workdir /tmp/oc-server`；凭据改为 `--token-file`（第 13 行）。
- `/etc/systemd/system/opencoder-agent@.service`：新增
  `ExecStartPre=/usr/bin/mkdir -p /tmp/oc-%i`（第 10 行），凭据改为
  `--token-file`（第 11 行）。
- 凭据文件：`/root/.opencoder/fleet-18080.token`（0600）；单元文件不再含
  明文 token。
- 重建 `/tmp/oc-server`、`/tmp/oc-node02`、`/tmp/oc-node04`。
- 验收实例 8091 固化为 `opencoder-server-acceptance.service`（enabled），
  运行 `target/release` 构建。
- 原单元备份：`/root/.opencoder/systemd-backup-20261125/`。

## 发布内容

- 构建：`cargo build --release -p opencoder-server -p opencoder-agent`，
  基线 `0b2016c5a`；两二进制 `--build-info` 一致：
  `0.1.1 (0b2016c5a-dirty)`、protocol 10、data_format 12/12。
- `-dirty` 说明：工作区仅 `tests/dag_e2e/` 两个测试文件未提交，不参与
  server/agent 编译产物；二进制源码等价于已提交的 `0b2016c5a`。
- 换装：`scripts/install.sh --source … --dest /usr/local/bin/… --no-build --backup`；
  回滚点 `opencoder-server.bak.20261125222047`、
  `opencoder-agent.bak.20261125222047`（原 0.1.0 d0e87cbca）。
- 产物 SHA256 前 16 位：server `0969f33fd142f0ac`、agent `100dec18a05f5525`。
- 滚动顺序：先 `systemctl restart opencoder-server` → 再
  `restart opencoder-agent@node02 @node04`。

## 验证

- `GET /api/health` → `ok:true`、`commit:"0.1.1 (0b2016c5a-dirty)"`、
  `protocol_version:10`、`role:"control"`；`GET /` 200、`/api/time` 200。
- `/api/nodes` → node02/node04 `online:true`、`status:idle`、
  `ready:true`、`resource_error:null`、`version:0.1.1`。
- 指纹稳定性：三次采样（间隔 20s）server/agent SHA256 不变。
- journal：换装后仅 2 条预期 `node channel failed`（agent 重启握手重置），
  无其他 ERROR。
- 自愈实测：`systemctl stop` → `rm -rf /tmp/oc-server` → `systemctl start`
  → `ExecStartPre` 自动重建目录，服务 active、`/api/time` 200。
- agent 自愈实测：重启 node02 时 `ExecStartPre` `status=0/SUCCESS`。
- SPA：`scripts/check-spa-drift.sh` → `spa dist: no drift`。
- 8091：持久单元 active/enabled，`GET /` 200、`/api/time` 200，
  版本 `0.1.1 (0b2016c5a-dirty)`。

## 测试覆盖

| 功能 | 测试名 / 命令 | 文件 |
|------|---------------|------|
| SPA 产物一致性 | `scripts/check-spa-drift.sh` | `scripts/check-spa-drift.sh` |
| 链接登录（?token/#token） | `node scripts/acceptance/link_login.js`（7 passed） | `scripts/acceptance/link_login.js` |
| 单元自愈 | stop → 删 workdir → start 实测 | 环境单元（无自动化测试） |
| 节点回连 | `GET /api/nodes` 全真 | 环境验收（无自动化测试） |
| 构建元数据一致 | `--build-info` 两二进制比对 | `crates/server/src/main.rs` |

- 定向回归：`scripts/check-spa-drift.sh` → no drift（exit 0）；
  `scripts/acceptance/link_login.js` → 7 passed / 0 failed。
- Clippy：未修改 Rust 代码，不适用。
- 测试范围：本次环境单元、发布换装与验收实例；未运行全量测试。

## Impact Surface

- `/usr/local/bin/opencoder-server`、`/usr/local/bin/opencoder-agent` 及两个
  systemd 单元；18080 Fleet 与 8091 验收实例。
- 仓库代码零改动；数据目录与 workdir 路径不变。

## Related Docs

- [phase-one 发布记录](../2026-11-21/phase-one-closure-memory-sync.md)
- [Agent 调度平台](../../platform/agent-platform/index.md)
