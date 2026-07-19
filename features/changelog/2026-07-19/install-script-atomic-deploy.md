# 部署：原子化安装脚本 scripts/install.sh + 清理漂移二进制

## 背景

仓库此前**没有任何 install/build 脚本**，`opencoder` 命令的「生效」完全靠手工 ad-hoc
操作，随时间已产生漂移。审查发现系统内同时存在三份不一致的二进制：

| 位置 | 类型 | 状态 |
|------|------|------|
| `/root/.cargo/bin/opencoder` | symlink → target/release | ✅ 实时跟踪 rebuild |
| `/usr/local/bin/opencoder` | hard copy（7月15） | ❌ 陈旧 4 天，MD5 `f511…` ≠ 最新 `d50c…` |
| `/data/caches/opencoder` | 孤儿副本（7月17） | ❌ 用途不明，三份不一致 |

需求：让 `cargo build --release` 之后，新二进制能**可靠、原子地**成为生效的
`/usr/local/bin/opencoder`（FHS 系统级路径，systemd / 绝对路径调用方依赖它），并且
即便 `opencoder` 正在运行也能安全替换（不报 ETXTBSY、不打断在跑的进程）。

## 变更

### 新增 `scripts/install.sh`（138 行）
- 原子安装流程：`cp` 到 `<dest>.new.<pid>` → `chmod 0755` → `sync`（fsync 暂存文件）
  → `mv -f` rename 覆盖目标。Linux `rename()` 原子换 inode，在跑的进程保留旧映射，
  不会触发 ETXTBSY、不会撕裂。
- 源二进制解析：`--source PATH` 显式指定；否则经 `cargo metadata --no-deps` 读取
  `target_directory`（自动遵循 `.cargo/config.toml` 的 `/data/caches/opencoder-target`），
  拼出 `release/opencoder`。无需硬编码路径。
- `--no-build` 跳过构建（默认会先 `cargo build --release`，失败返回 exit 2）。
- `--dest PATH` / `OPENCODER_INSTALL_DEST` 覆盖目标路径（默认 `/usr/local/bin/opencoder`），
  便于测试与多环境部署。
- 安装后**自检**：`<dest> --version` 必须退出 0，且版本串与源一致，否则 exit 5。
- trap 清理暂存文件，杜绝 `*.new.*` 残留。
- 选 hard copy 而非 symlink：`/usr/local/bin` 在 `/data` 缓存被清时仍须可用，绝对路径
  调用方不应跟随可能 dangling 的软链。`~/.cargo/bin` 软链保留给交互式开发，自动跟踪 rebuild。

### 新增 `scripts/e2e/test_install.sh`（99 行）
- install.sh 的契约测试，**不碰系统路径、不需 LLM API key**（mktemp 工作目录）。
- 6 条可观测契约 C1–C6：退出码、可执行位、版本一致、幂等（md5 稳定）、无暂存残留、
  `--source` 覆盖安装给定二进制。

### 现场清理（本次执行）
- `scripts/install.sh --no-build` 把 `/usr/local/bin/opencoder` 刷新为最新（`f511…` → `d50c…`）。
- 删除孤儿副本 `/data/caches/opencoder`。
- 三处生效路径现 MD5 一致：`d50c9399…`（cargo bin symlink / target release / usr-local hard copy）。
- 验证 `opencoder`（pid 91692）在原子替换期间**未被打断**。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| install.sh 原子安装退出 0 | C1 install.sh exits 0 | `scripts/e2e/test_install.sh` |
| 安装产物可执行位正确 | C2 installed file exists and is executable | `scripts/e2e/test_install.sh` |
| 安装版本与源一致 | C3 installed --version matches source | `scripts/e2e/test_install.sh` |
| 重复安装幂等 | C4 idempotent (md5 stable across two installs) | `scripts/e2e/test_install.sh` |
| 无暂存文件残留 | C5 no atomic-staging leftovers | `scripts/e2e/test_install.sh` |
| --source 覆盖生效 | C6 --source override installs the given binary verbatim | `scripts/e2e/test_install.sh` |

> 说明：本次变更为部署/安装脚本，不动 Rust 业务代码，故 `cargo test --workspace`
> 用例数与基线持平（625）。新增覆盖落在 shell 契约测试层（`scripts/e2e/test_install.sh`，
> 6 passed / 0 failed），属 Rule 03 的 L3 旁路测试（无需 API key、确定性）。

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 625 passed / 0 failed / 0 ignored（业务码未改动，与基线一致）|
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --release` | 零错误（11.6 MB，`opencoder 0.1.0`）|
| `scripts/e2e/test_install.sh` | 6 passed / 0 failed |
| 三处生效路径 MD5 一致 | `d50c9399…`（cargo bin / target release / usr-local）|

## Impact Surface

- 新增：部署脚本 `scripts/install.sh`、契约测试 `scripts/e2e/test_install.sh`。
- 现场修复：`/usr/local/bin/opencoder` 刷新为最新；移除 `/data/caches/opencoder` 孤儿副本。
- 不影响：任何 Rust crate 源码、`Store` / `ChatStream` 抽象、session/web/cli/tui 业务行为。
- 不影响：`opencode`（旧 167MB 二进制）及依赖它的 `opencode-vps-bootstrap.sh` / systemd tunnel——
  本次按既定决策**不替换** `opencode`。

## 运维说明

日常发版后让新二进制生效：

```bash
scripts/install.sh                 # = cargo build --release + 原子装到 /usr/local/bin
scripts/install.sh --no-build      # 已构建好，仅刷新 /usr/local/bin
scripts/e2e/test_install.sh        # 跑契约自检（无 API key，秒级）
```

绝对路径调用方（systemd 等）现统一走 `/usr/local/bin/opencoder`，由 install.sh 保活；
交互式开发继续用 PATH 命中的 `~/.cargo/bin/opencoder`（symlink，自动跟踪 rebuild）。
