Commit: (working-tree)

# aarch64 交叉编译配置落地(无代码改动)

- 新增 `.cargo/config.toml`:`[target.aarch64-unknown-linux-gnu]` 指定交叉链接器 `aarch64-linux-gnu-gcc`,`[env]` 提供 `CC/AR/CXX_aarch64_unknown_linux_gnu` 供 C 依赖(libsql-sqlite3、ring)交叉编译;零源码改动。
- 产物验证:`cargo build --release --target aarch64-unknown-linux-gnu -j 4 -p opencoder-agent -p opencoder-cli` 成功(6m09s),`file` 均为 ARM aarch64 stripped;agent 34.3 MiB / cli 5.1 MiB,位于 `/data00/rust-build/cargo/default/aarch64-unknown-linux-gnu/release/`;GLIBC 符号最高 2.28,满足 Ubuntu 24.04 (2.39)。
- 完整命令、宿主坑(guru canary 内存竞争→强制 `-j`、nohang-watchdog、E0463 陷阱)沉淀在 [docs/cross-compile-aarch64.md](../../../docs/cross-compile-aarch64.md)。
