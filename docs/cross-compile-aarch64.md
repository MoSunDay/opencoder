Commit: (working-tree)

# aarch64 Linux 交叉编译(仅构建配置层,零源码改动)

面向 Ubuntu 24.04 / glibc 2.39 的 ARM64 目标二进制(`opencoder-agent`、`opencoder-cli`)。本仓库已提供 `.cargo/config.toml`(未跟踪、可随仓库分发),交叉编译不需要任何源码或依赖改动。

## 前置条件(宿主机已就绪)

- rustup target:`aarch64-unknown-linux-gnu` 已装在**默认 toolchain**(`stable-x86_64-unknown-linux-gnu`,即 rustc 1.98.0)。
  - 注意:`rustup target list --installed` 是 per-toolchain 的;`1.98.0-*` 目录副本只装了 x86_64 std,用它的绝对路径 cargo 会报 E0463(can't find crate for target)。经 rustup shim 或 stable 绝对路径均正确。
- 交叉链接器/归档器:`aarch64-linux-gnu-gcc`、`aarch64-linux-gnu-ar`(binutils 2.31 cross,系统已装)。
- 环境由 `~/.config/build-cache-env.sh` 注入:`CARGO_HOME=/data00/rust-build/cargo-home`、`RUSTUP_HOME=/data00/rust-build/rustup-home/.rustup`、`CARGO_TARGET_DIR=/data00/rust-build/cargo/default`(共享 target,产物不在 repo `target/` 下)。

## 仓库内配置(.cargo/config.toml)

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"

[env]
CC_aarch64_unknown_linux_gnu = "aarch64-linux-gnu-gcc"
AR_aarch64_unknown_linux_gnu = "aarch64-linux-gnu-ar"
CXX_aarch64_unknown_linux_gnu = "aarch64-linux-gnu-g++"
```

`[env]` 段供 C 依赖(`libsql-sqlite3`、`ring` 等)的 cc 构建脚本交叉编译用;不用环境变量方式时删除 `[env]` 也不影响 Rust 部分。

## 编译命令

```bash
# 登录 shell 已带 CARGO_HOME/RUSTUP_HOME/CARGO_TARGET_DIR
cargo build --release --target aarch64-unknown-linux-gnu -j 4 \
  -p opencoder-agent -p opencoder-cli
```

- `-p` 仅构建 agent(cli)+ 依赖子图,跳过 server/tui 等不必要组件;workspace 根全量构建也可。
- `-j 4` 是刻意压低:宿主常有高内存负载(见"已知坑"),并发 rustc 峰值会触发全局 OOM。
- 首次全量约 6-8 分钟(-j 4);`.cargo/config.toml` 与 `Cargo.lock` 同目录生效。

## 产物与验证

| 产物 | 绝对路径 | 大小 | file |
| --- | --- | --- | --- |
| agent | `/data00/rust-build/cargo/default/aarch64-unknown-linux-gnu/release/opencoder-agent` | 35,963,392 B (34.3 MiB) | ELF 64-bit LSB shared object, ARM aarch64, dynamically linked, stripped |
| cli | `/data00/rust-build/cargo/default/aarch64-unknown-linux-gnu/release/opencoder-cli` | 5,339,480 B (5.1 MiB) | 同上 |

- 依赖符号版本最高 `GLIBC_2.28`(libgcc_s/libpthread/libm/libdl/libc),满足 glibc 2.39 目标。
- 链接期 warning `unsupported GNU_PROPERTY_TYPE (5) type: 0xc0000000` 来自旧版交叉 ld(2.31)对新 rustlib note 的告警,无害可忽略。

## 遇到的坑(宿主环境)

- **内存竞争**:宿主有 guru canary(`systemd-run -p MemoryMax=190G`)与此前 global OOM 记录(20:50,`opencoder` 进程 anon-rss 154GB 被杀);构建必须限制 `-j`,建议 `systemd-run --scope -p OOMScoreAdjust=-500 -p MemoryMax=80G` 隔离运行,避免被 OOM/会话清理波及。
- **nohang-watchdog**(`/root/tools/nohang-watchdog.sh`):每 60s 巡检,运行超 300s 的 `find/grep/rg` 一律 SIGTERM;与构建无冲突,但排查日志时不要裸跑大范围 find/grep。
- **僵尸进程**:构建被杀后 `cargo/rustc` 可能挂为 zombie(subreaper `sleep infinity` 不收尸),不代表仍在编译;以 `systemctl is-active`/日志 `Finished` 为准。
- **日志分流**:`systemd-run` 方式下 cargo 输出进 journal(`journalctl -u <unit>`),不会进 stdout 重定向文件。

## 2026-09-17 实际执行记录

- 命令:`cargo build --release --target aarch64-unknown-linux-gnu -j 4 -p opencoder-agent -p opencoder-cli`(经 `systemd-run --unit=oc-aarch64-build` 独立 scope)。
- 结果:`Finished release profile [optimized] target(s) in 6m 09s`;两产物 file 均为 `ARM aarch64`,见上表。
- 变更面:仅新增 `.cargo/config.toml` 与本文档,无源码/依赖改动。
