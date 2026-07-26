Commit: (working-tree, pre-initial-commit)

# fix(session/bash): hide `timeout` from the model-facing tool schema

## 背景

继 `bash-timeout-clamp-handoff.md`（clamp + handoff 修复）之后，schema 里
仍保留了可写的 `timeout` 属性。这给模型留了一条「主动放大 timeout」的路径——
即便 `resolve_timeout_secs` 在运行期会 `.min(590)` 兜底，模型仍会频繁尝试传
超高 `timeout`（历史 ~25% 的调用），产生无谓的 token 与歧义。

## 修复

`crates/session/src/tools/bash.rs::parameters()`：删除 `timeout` 属性，模型
可见的 schema 仅剩 `command` / `workdir`。

- **执行逻辑零改动**：`resolve_timeout_secs(&input)` 仍读 `input.get("timeout")`
  （模型不传时 `unwrap_or(120).min(BASH_MAX_TIMEOUT_SECS)` 兜底），handoff 路径、
  `kill_on_drop`、`setsid()`、动态「moved to background」提示全部保留。
- **不变量不变**：`BASH_MAX_TIMEOUT_SECS = 590` 与 `DEFAULT_TOOL_TIMEOUT = 600s`
  的编译期断言（cap < 安全网）不受影响。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| schema 不暴露 `timeout`（新增回归） | `parameters_schema_hides_timeout_from_model` | `crates/session/src/tools/bash.rs` |
| schema 保留 `command` / `workdir` | `parameters_schema_hides_timeout_from_model` | `crates/session/src/tools/bash.rs` |
| timeout clamp：默认值 / sub-cap / ≥600 截断 | `timeout_clamped_below_safety_net` | `crates/session/src/tools/bash.rs` |
| 编译期断言：cap < DEFAULT_TOOL_TIMEOUT | `const _: () = assert!(...)` | `crates/session/src/tools/bash.rs` |
| handoff 机制（运行期不变） | `bash_handoff_on_timeout` | `crates/session/src/tools/bash.rs` |
| handoff 进程存活（契约） | `bash_tool_hands_off_on_timeout` | `crates/session/tests/tools_contract.rs` |
| handoff 输出文件（契约） | `bash_tool_output_file_captures_output_on_timeout` | `crates/session/tests/tools_contract.rs` |

- 全量回归：`cargo test --workspace` → **1205 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：bash.rs 385 行（< 800）
