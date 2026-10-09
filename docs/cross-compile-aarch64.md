Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# aarch64 Linux 交叉编译

OpenCoder 的 `opencoder-agent` 和 `opencoder-cli` 可以面向 `aarch64-unknown-linux-gnu` 构建。构建使用的交叉工具链和目标系统的 glibc 必须匹配。

## 前置条件

为实际使用的 Rust toolchain 安装目标标准库：

```bash
rustup target add aarch64-unknown-linux-gnu
rustup target list --installed
```

宿主机需要 `aarch64-linux-gnu-gcc`、`aarch64-linux-gnu-g++` 和 `aarch64-linux-gnu-ar`。目标列表按 toolchain 分开，检查和构建应使用同一 toolchain。

## 本地构建配置

在本地创建 `.cargo/config.toml`；此文件不随仓库提交：

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"

[env]
CC_aarch64_unknown_linux_gnu = "aarch64-linux-gnu-gcc"
AR_aarch64_unknown_linux_gnu = "aarch64-linux-gnu-ar"
CXX_aarch64_unknown_linux_gnu = "aarch64-linux-gnu-g++"
```

链接器配置供 Rust 使用；`CC`、`AR` 和 `CXX` 供 C/C++ 依赖的构建脚本使用。

## 构建与产物检查

```bash
cargo build --release --target aarch64-unknown-linux-gnu \
  -p opencoder-agent -p opencoder-cli
file target/aarch64-unknown-linux-gnu/release/opencoder-agent
file target/aarch64-unknown-linux-gnu/release/opencoder-cli
```

默认产物位于 `target/aarch64-unknown-linux-gnu/release/`。配置了其他 target 目录时，在对应目录检查产物。`file` 应显示 ARM aarch64 ELF；部署前还需确认产物依赖的 glibc 符号版本不高于目标系统提供的版本。

构建并发可通过 Cargo 的 `-j` 参数按宿主资源调整。
