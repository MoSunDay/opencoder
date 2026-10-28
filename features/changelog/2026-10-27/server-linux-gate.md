Commit: 5f44796ab1344a93f573b48af2d39929fdc27c9f
Commit: e0abec72ef0412a755956277a08eef737c4f9add

# opencode-server 运行时限制为 Linux

`opencode-server` 现在在非 Linux 平台启动时直接拒绝运行：参数解析和 `--build_info` 保持全平台可用，之后主入口在任何服务工作开始前检查平台，非 Linux 平台向 stderr 输出说明（指明需要 Linux、TUI/CLI 仍跨平台）并以非零码退出。Linux 路径行为不变。

该限制是运行时拒绝而不是编译错误：三个平台族（Linux/macOS/Windows）都能继续编译该二进制，跨平台 TUI 开发不受影响。平台判断提取为纯函数 `platform_supported` 与 `unsupported_platform_error`，无配置项、协议字段或持久化格式变化。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 平台门与编译目标一致且确定性；错误信息指明 Linux、跨平台 TUI 与当前平台 | `platform_gate_is_deterministic_and_message_names_linux_and_tui` |

## 验证范围

仅运行本次改动及直接影响链路的定向检查，未运行全量测试。

| 实际命令 / 目标 | 结果 |
| --- | --- |
| `cargo check -p opencoder-server` | 通过，零警告 |
| `cargo clippy -p opencoder-server --all-targets` | 通过，零警告 |
| `cargo test -p opencoder-server --bin opencoder-server` | 5 项通过（含新增门测试） |
| `cargo check --target aarch64-apple-darwin -p opencoder-server` | 环境缺少 arm64-darwin C 交叉编译器，在依赖 `ring` 的构建脚本失败；HEAD 基线同样失败，属既有环境限制，非本次改动引入 |

作为 macOS 可编译性的补充验证，将新增的两个纯函数原样提取后用 `rustc --target aarch64-apple-darwin --crate-type lib --emit=metadata` 类型检查通过；代码仅使用 `cfg!`、`std::env::consts::OS`、`eprintln!`、`std::process::exit` 等平台无关标准库接口，未引入任何平台条件编译分支。

相关说明：[Agent 调度平台](../../../docs/agent-platform.md)。
