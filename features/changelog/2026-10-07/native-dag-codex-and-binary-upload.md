# 原生 DAG 的 Codex 结束行为和二进制上传

Codex 完成后不再启动使用原生模型配置的 Act 记忆会话，避免应用已经结束却因未配置原生模型而报错；Codex 自身失败仍保留失败结果。

发布生成的 Nginx 配置对 Server 二进制池上传路径使用 48 MiB 请求上限，容纳最大 32 MiB ELF 的 Base64 JSON。其他 Server 请求和 Host 入口仍为 2 MiB。升级和回滚都会生成相同的路径限制。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| Codex 完成不派生 Act 会话 | `codex_completion_does_not_start_an_act_memory_session` | `crates/session/tests/harness_memory.rs` |
| Codex 失败保持失败及原始证据 | `codex_failure_stays_failed_with_native_memory_enabled` | `crates/session/tests/harness_memory.rs` |
| 大请求限制只作用于 Server 二进制池上传 | `test_binary_upload_limit_is_scoped_to_server_resource_routes` | `scripts/platform/rolling_tests/test_deployment.py` |

平台发布脚本 63 项测试通过；`cargo clippy --workspace --all-targets -- -D warnings` 零警告，`cargo test --workspace` 共 5,679 项通过、0 项失败、7 项既有忽略，`cargo build --workspace` 通过。测试进程文件句柄上限为 65,536；默认上限曾导致控制器集成测试连接失败，调整后完整重跑通过。真实发布演练另行保存回执。
