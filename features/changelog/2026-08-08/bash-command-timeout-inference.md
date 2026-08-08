Commit: 05d4bdf110cd7bfa75492f8ea7eebbb7cdb4c662

# Bash 命令超时推导兼容扩展

## Context

既有 bash 工具支持静默前缀 `timeout N; command`：前缀只表达 OpenCoder 的前台
deadline，执行前必须剥离。实际 GNU `timeout N command` 或较长 `sleep N` 出现在复合
命令中时，旧逻辑无法据此放宽 deadline，可能过早 handoff。

## Change Summary

- 保留并优先识别静默 `timeout N; command` 契约，继续剥离前缀；生产下维持既有
  120–600 秒范围，测试下保留 1 秒下限以快速验证 handoff。
- 对非静默前缀命令，只识别 shell statement 的 command position 上未引用的
  `timeout N` / `sleep N`；原命令不改写。
- `timeout N` 使用 30–600 秒；`sleep N` 先把 N 限制到 30–600，再完整加 120 秒，
  最终范围为 150–720 秒。多个同类 command 取最大值，timeout 优先于 sleep。
- 引号、注释和普通参数中的提示词不参与推导，避免日志文本把前台占用误延长到
  600 秒；扫描无正则、无分配。
- 空静默前缀 `timeout N;` 继续返回 `empty command`，工具 schema 和默认
  130 秒真实 deadline / 120 秒展示值不变。

## Compatibility

兼容 2026-08-07 已发布的静默前缀行为，同时扩展真实 GNU timeout 与 sleep 场景；无
数据库、配置、环境变量或公开 schema 变化。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 静默前缀剥离与兼容下限 | `legacy_prefix_is_stripped_and_uses_compatible_test_floor` | `tools/bash/timeout.rs` |
| 静默前缀触发 handoff | `legacy_timeout_prefix_triggers_handoff_and_is_not_executed` | `tools/bash.rs` |
| 静默前缀放宽默认 deadline | `legacy_timeout_prefix_widens_default_test_deadline` | `tools/bash.rs` |
| 空静默前缀报错 | `legacy_timeout_prefix_with_empty_rest_errors` | `tools/bash.rs` |
| GNU timeout、sleep 与上限 | `real_timeout_is_clamped_and_command_is_retained` / `largest_command_position_hint_wins` | `tools/bash/timeout.rs` |
| sleep 的最小/最大输入与完整 120 秒余量 | `sleep_clamps_x_then_adds_the_full_padding` / `huge_values_saturate_then_cap` | `tools/bash/timeout.rs` |
| 引号、注释与普通参数不误匹配 | `quotes_comments_and_arguments_do_not_extend_timeout` | `tools/bash/timeout.rs` |
| 子 shell/brace 结束符可终止扫描且不死循环 | `grouped_commands_terminate_the_scan` / `plan_mode_allows_subshell_fd_merge` | `tools/bash/timeout.rs` / `tests/bash_guard_plan_mode.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- 行数：新增 `tools/bash/timeout.rs` ≤ 400；本轮修改文件均 ≤ 800。

## Related Docs

- [session 模块](../../../agents/session/index.md)
