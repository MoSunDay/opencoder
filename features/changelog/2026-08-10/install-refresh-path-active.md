Commit: 0e0ec867c45170ffb244e38469baf7f4508bacc9

# 部署：刷新三处 opencoder 生效路径至 `0.1.0 (1648be8-dirty)`

## 背景

审计发现系统内三份 `opencoder` 二进制不一致，**PATH 实际生效位**仍是陈旧副本：

| 位置 | 部署前版本 | 说明 |
|------|-----------|------|
| `/data00/rust-build/cargo/default/release/opencoder` | `0.1.0 (1648be8-dirty)`（19:03） | cargo target-dir 下最新 release 构建 |
| `/root/.local/bin/opencoder` | `0.1.0 (1648be8)`（17:40） | **PATH 实际解析位**（`/root/.local/bin` 在 PATH 首位），陈旧 |
| `/usr/local/bin/opencoder` | `0.1.0 (246e2aa)`（08:39） | FHS 规范位，更陈旧 |

需求：把最新 release 构建原子安装到 PATH 生效位 `/root/.local/bin/opencoder`，
并同步刷新 FHS 位 `/usr/local/bin/opencoder`，使新开 shell 里 `opencoder` 即当前代码。

## 执行

完全复用既有 `scripts/install.sh` 原子安装机制（cp → fsync → rename），无新增脚本、
无任何 Rust 源码改动。工作树含未提交 TUI 改动，故构建带 `-dirty` 后缀（与 19:03 构建一致）。

1. `scripts/install.sh --dest /root/.local/bin/opencoder --backup`
   → 构建 release（49.97s）+ 安装到 PATH 生效位；旧副本备份为 `opencoder.bak.20260810191519`。
2. `scripts/install.sh --dest /usr/local/bin/opencoder --backup --no-build`
   → 复用已构建产物刷新 FHS 位；旧副本备份为 `opencoder.bak.20260810191520`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| install.sh 原子安装退出 0 | C1 install.sh exits 0 | `scripts/e2e/test_install.sh` |
| 安装产物可执行位正确 | C2 installed file exists and is executable | `scripts/e2e/test_install.sh` |
| 安装版本与源一致 | C3 installed --version matches source | `scripts/e2e/test_install.sh` |
| 重复安装幂等 | C4 idempotent (md5 stable across two installs) | `scripts/e2e/test_install.sh` |
| 无暂存文件残留 | C5 no atomic-staging leftovers | `scripts/e2e/test_install.sh` |
| --source 覆盖生效 | C6 --source override installs the given binary verbatim | `scripts/e2e/test_install.sh` |
| --backup 保留旧目标 | C7 --backup saved prior destination as .bak | `scripts/e2e/test_install.sh` |
| --backup 目标不存在时不建备份 | C8 --backup on non-existent dest | `scripts/e2e/test_install.sh` |

## Gate

| 项 | 结果 |
|----|------|
| `which opencoder` | `/root/.local/bin/opencoder` |
| `opencoder --version` | `opencoder 0.1.0 (1648be8-dirty)` |
| 三处生效路径 MD5 | `4bb0a3d43cb93e991f4ae8c887666a4d` 全一致 |
| `scripts/e2e/test_install.sh` | 8 passed / 0 failed（C1–C8） |
| `*.new.*` 暂存残留 | 无 |

## Impact Surface

- 现场刷新：`/root/.local/bin/opencoder`（PATH 生效位）、`/usr/local/bin/opencoder`（FHS 位）。
- 备份：`/root/.local/bin/opencoder.bak.20260810191519`、`/usr/local/bin/opencoder.bak.20260810191520`
  （均不覆盖历史备份）。
- 不影响：任何 Rust crate 源码、`Store` / `ChatStream` 抽象、session/web/cli/tui 业务行为。
- 不影响：`opencode`（旧二进制）及依赖它的 systemd 隧道。
