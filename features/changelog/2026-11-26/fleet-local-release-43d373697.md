Commit: 43d373697

# Fleet 本机发布 43d373697：DAG 沙箱宿主映射与 SPA 调整上到 18080 与 8091

## 背景

- 用户要求把含全部已提交变动的干净提交发布到本机两套实例，生效逻辑由
  用户验收。
- `43d373697` 含两项变动：
  - `e0a7113ac` DAG 沙箱宿主映射 + 配置键 `dag.runners_dir`；
  - `43d373697` SPA：品牌菜单分类标题补图标、项目 / 子项表移除「现状」列。
- 发布前：18080 Fleet 跑 `0b2016c5a-dirty`；8091 验收实例跑
  `6f6a95e6b-dirty`（11-25 22:41 的本地构建，早于 DAG 变更）。

## 变更

- 仓库代码零改动，仅新增本记录。
- 环境侧换装三个二进制、重启四个单元；单元文件、workdir、数据目录、
  token 文件均未变。
- 未换装：`/usr/local/bin/opencoder`（交互会话运行中）、
  `/usr/local/bin/opencoder-cli`——本轮改动不涉及。
- runner（`dag-runner`、`agent-step-runner`、`agent-session-runner`）随包
  留在发布目录，未装到节点：全局 `config.json` 无 `dag` 段，DAG 未配置。

## 发布内容

- 构建：`scripts/platform/release/build.sh`，独立 worktree（detached
  `43d373697`）+ 独立 `CARGO_TARGET_DIR`，exit 0。脚本要求干净工作区、
  内置 `scripts/check-spa-drift.sh`（`spa dist: no drift`），并以
  `OPENCODER_BUILD_COMMIT` + `OPENCODER_SPA_SHA256` 注入构建元数据。
- 元数据：7 个二进制 `--build-info` 一致：`0.1.1 (43d373697)`、
  `git_dirty:false`、protocol 10、data_format 12/12、brain_schema 7、
  `spa_sha256 825db945d4332565…`。
- 发布包：`dist/opencoder-platform-43d373697/`（`bin/` 7 个二进制 +
  `manifest.json` + `SHA256SUMS`，129 MB，位于构建 worktree 的易失目录）；
  `sha256sum -c SHA256SUMS` 8 项通过。
- 产物 SHA256 前 16 位：server `62e81f176041b45a`、agent `3db47526845c0546`
  （完整值见包内 `SHA256SUMS`）。
- 换装：`scripts/install.sh --source <包>/bin/<name> --dest <目标路径>
  --no-build --backup`（临时文件 + rename，自检 `--version`）。
- 回滚点：
  - 18080：`/usr/local/bin/opencoder-server.bak.20261126205924`、
    `/usr/local/bin/opencoder-agent.bak.20261126205924`
    （原 `0b2016c5a-dirty`）；
  - 8091：`/root/opencoder/target/release/opencoder-server.bak.20261126210259`
    （原 `6f6a95e6b-dirty`）。
- 回滚方法：`scripts/install.sh --source <对应 .bak> --dest <原路径> --no-build`，
  再按同样顺序 restart。
- 滚动顺序：先 `systemctl restart opencoder-server`（20:59:25）→
  `restart opencoder-agent@node02 @node04`（20:59:38）→ 节点回连确认后
  `restart opencoder-server-acceptance`（21:02:59）。
- 与主分支的关系：发布后 `main` 前进到 `5eaf44462`，相对 `43d373697` 仅
  2 个 vitest 文件、1 个验收脚本、1 个 changelog，无运行时路径；运行时与
  本次产物等价，未重切。

## 验证

- 换装前检查：18080 无未终结执行、两节点 idle；8091 `/api/executions` 为空，
  无浏览器 / 验收 / 构建进程，无已建立连接。
- 18080：`GET /api/health` → `ok:true`、`commit:"0.1.1 (43d373697)"`、
  `protocol_version:10`、`role:"control"`。
- `/api/nodes` 三次采样 → node02 / node04 `online:true`、`status:idle`。
- 8091：单元 active、`NRestarts=0`；`GET /` 200、`/api/time` 200；
  `/api/health` commit 同上；运行进程 exe 为预期路径。
- 已装二进制：server `--build-info` 的 `git_commit` 为 `43d373697…`、
  `git_dirty:false`、`spa_sha256 825db945…`；三个已装二进制 SHA256 与
  发布包一致。
- 指纹稳定性：三次采样（21:03:46 / 21:04:07 / 21:04:27，间隔约 20s）三个
  已装二进制 SHA256 不变，四个单元均 active。
- journal：换装后仅 2 条预期 `node channel failed`（agent 重启握手重置），
  `-p err` 无条目；6 条 `migrated domain keys` WARN 来自全局 `config.json`，
  换装前后条数不变；8091 重启后日志无 ERROR / WARN。

## 测试覆盖

| 功能 | 测试名 / 命令 | 文件 |
|------|---------------|------|
| 发布构建与元数据一致 | `scripts/platform/release/build.sh`（`--build-info` 逐项校验） | `scripts/platform/release/build.sh` |
| 产物完整性 | `sha256sum -c SHA256SUMS` | 发布包 `SHA256SUMS` |
| SPA 产物一致性 | `scripts/check-spa-drift.sh`（构建脚本内置） | `scripts/check-spa-drift.sh` |
| 换装原子性与自检 | `scripts/install.sh … --no-build --backup` | `scripts/install.sh` |
| 控制面健康与节点回连 | `GET /api/health`、`/api/nodes`、`/api/executions` | 环境验收（无自动化测试） |
| 指纹稳定性 | 三次采样 SHA256 比对 | 环境验收（无自动化测试） |
| DAG 沙箱宿主映射（所含变动） | 见该记录测试表 | [dag-sandbox-host-mapping.md](../2026-11-25/dag-sandbox-host-mapping.md) |
| SPA 导航与表格调整（所含变动） | 见该记录测试表 | [fleet-nav-group-icons-summary-out-of-tables.md](../2026-11-25/fleet-nav-group-icons-summary-out-of-tables.md) |

- 定向回归：本次未改代码，未重跑所含变动的测试（提交后代码未再变化，
  结果见各自记录）；发布期检查 `sha256sum -c SHA256SUMS` → 8 通过。
- Clippy：不适用（未修改 Rust 代码）。
- 测试范围：发布构建、换装、两套本机实例的健康与日志；未运行全量测试。

## Impact Surface

- 18080 Fleet（`/usr/local/bin/opencoder-server` + 两节点
  `/usr/local/bin/opencoder-agent`）与 8091 验收实例
  （`/root/opencoder/target/release/opencoder-server`）的运行二进制；
  三个回滚点文件。
- 仓库代码零改动；单元文件、workdir、数据目录、token 文件不变。
- 注意：8091 直接运行主工作区 `target/release/` 下的二进制，而主工作区的
  构建缓存停留在 DAG 变更之前。此后在主工作区执行 `cargo build --release`
  会重建 server 并覆盖该文件为非发布构建（`spa_sha256` 为 unknown、commit
  为当时 HEAD）；验收前以 `/api/health` 的 `commit` 为准。

## Related Docs

- [DAG 沙箱宿主映射](../2026-11-25/dag-sandbox-host-mapping.md)
- [品牌菜单分类图标与现状列移除](../2026-11-25/fleet-nav-group-icons-summary-out-of-tables.md)
- [Fleet 本机发布 0b2016c5a 与 18080 workdir 自愈](../2026-11-25/fleet-local-release-workdir-selfheal.md)
