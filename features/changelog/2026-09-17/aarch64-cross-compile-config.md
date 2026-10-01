Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# aarch64 交叉编译配置

OpenCoder Agent 和 CLI 使用 `aarch64-unknown-linux-gnu` 目标构建。Rust 链接器由本地 Cargo 配置指定，C/C++ 依赖使用对应的 `CC`、`AR` 和 `CXX`；无需修改源码或依赖。

当时执行 `cargo build --release --target aarch64-unknown-linux-gnu -j 4 -p opencoder-agent -p opencoder-cli` 成功，`file` 确认两份产物均为 ARM aarch64 ELF。实际机器的产物路径、大小和资源争用记录已外移。

[构建配置和产物检查](../../../docs/cross-compile-aarch64.md)
