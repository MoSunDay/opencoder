Commit: 347a6bdfee28570f5c5cf9e2e1891d563cdf1bb7

# Cua 原生模型接入与 Windows 验证

可选 CLI 支持通过 Cua 通用视觉循环使用 GLM-5.3-flash。桌面模型配置独立于 OpenCoder 主模型；原生函数模式补充当前截图和归一化坐标说明，执行前校验响应格式。模型返回混合工具文本或非法参数时明确失败，避免把未执行的动作当成完成。

Windows 输入使用 Cua 的剪贴板与粘贴命令保留汉字，并向模型说明原生滚动使用滚轮刻度。增加 `doctor --check-model`，只预测视觉描述与合成图片上的点击，不执行桌面动作。

CLI 注册说明要求前台启动、保留自动后台管理句柄、使用全新输出目录，并按截图或保存文件核对结果。独立 Windows 安装脚本及转发脚本提供受保护的部署与回退；恢复连接不重放任务。接入范围为本机 OpenCoder CLI/TUI。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 原生函数定义与 Cua 坐标转换 | `test_native_tool_request_executes_cua_converted_coordinates` | [test_native_model.py](../../../tools/computer-use/tests/test_native_model.py) |
| 只读模型检查、错误坐标及空视觉响应 | `test_model_probe_checks_coordinates_without_desktop_actions`、`test_model_probe_rejects_raw_pixel_coordinates`、`test_model_probe_rejects_empty_vision_response` | [test_native_model.py](../../../tools/computer-use/tests/test_native_model.py) |
| 非法工具响应停止执行 | `test_native_malformed_response_stops_before_any_action` | [test_native_model.py](../../../tools/computer-use/tests/test_native_model.py) |
| Windows 中文粘贴与能力拒绝 | `test_windows_unicode_typing_uses_cua_clipboard_and_preserves_refusal` | [test_native_model.py](../../../tools/computer-use/tests/test_native_model.py) |
| 模型检查 CLI 与配置边界 | `test_cli_model_probe_outputs_ready_without_input_actions`、`test_native_tool_mode_rejects_non_generic_cua_loop`、`test_native_tool_mode_requires_boolean_config` | [test_native_model.py](../../../tools/computer-use/tests/test_native_model.py) |
| 超时、取消、动作上限、连接关闭与互斥 | `test_timeout_closes_connections_and_releases_target`、`test_cancel_during_model_wait_stops_and_persists`、`test_action_limit_stops_before_next_remote_action`、`test_concurrent_runs_cannot_control_same_target` | [test_sdk.py](../../../tools/computer-use/tests/test_sdk.py) |
| CLI 取消及信号退出 | `test_cancel_cli_stops_running_process`、`test_sigterm_persists_cancelled_result` | [test_cli.py](../../../tools/computer-use/tests/test_cli.py) |
| 认证头和分段诊断凭证遮盖 | `test_authentication_errors_redact_tokens_without_the_header_scheme`、`test_diagnostics_preserve_json_stdout_and_redact_split_secrets` | [test_results_state.py](../../../tools/computer-use/tests/test_results_state.py) |
| 主代理注入、禁用配置 | `computer_cli_example_loads_and_injects_into_primary_agent`、`disabled_computer_cli_is_not_injected` | [computer_cli_registration.rs](../../../crates/core/tests/computer_cli_registration.rs) |

## 验证结果

Rust 回归在基线 `347a6bdf` 加本次 CLI 注册测试的隔离候选目录执行。测试所需 Server、节点及容器 runner 使用一致的构建信息；没有修改 DAG 版本检查。

- Python：63 passed / 0 failed，包含真实 Cua SDK 与 CLI 子进程；1 条上游 DashScope 废弃警告。
- `ruff check tools/computer-use`：通过。
- `cargo test --workspace`：退出码 0；逐目标汇总 **5595 passed / 0 failed / 7 ignored**，忽略项为仓库已有用例。
- `cargo build --workspace`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。

实际输出摘录：

```text
63 passed, 1 warning in 116.27s (0:01:56)

Running tests/computer_cli_registration.rs
test disabled_computer_cli_is_not_injected ... ok
test computer_cli_example_loads_and_injects_into_primary_agent ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

实际 Windows 桌面经 OpenCoder 注册入口验证 6 个样本：计算器结果、中文文件保存并重新打开、浏览器单次点击与滚动各 2 轮，均独立核对截图或文件内容。真实桌面链路上的故障注入覆盖取消、超时、SIGINT、异常退出、互斥、目录复用、错误凭证及断线恢复；取消约 1.5 秒停止，恢复未重放旧任务。

最终固定运行产物完成 15 分钟观察，16 个检查点通过；观察前后真实模型检查通过。macOS/Linux 原生桌面未在该环境验证。

## 相关索引

- [computer-use 实现](../../../agents/computer-use/index.md)
- [computer-use 能力](../../computer-use/index.md)
- [安装与配置](../../../tools/computer-use/README.md)
