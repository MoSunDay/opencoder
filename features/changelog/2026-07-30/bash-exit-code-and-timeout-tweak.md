# bash 工具：成功不输出退出码 + 前台运行/register-unregister 架构

## 背景

两条针对 bash 工具的改动：

1. **退出码**：此前无论成功失败都在输出末尾附 `[exit code: N]`。成功（`code == 0`）时退出码是冗余噪音——成功本身已隐含退出码 0；只有失败（`code != 0`）才需要显式标注，让模型看到失败并作出反应。前台执行路径（`bash.rs`）与后台 supervisor 路径（`bg.rs` legacy `handoff`）都改。
2. **前台运行 + register/unregister**：bash 命令不再自带超时和后台交接（handoff）。命令在前台运行直到自然退出；spawn 时 `register(pid, pgid, session_id)` 到全局注册表，完成后 `unregister(pid)`。`/ps` 可列出运行中命令，`/stop` 可按 pid 杀进程组。旧 `handoff` 路径保留但不再从前台工具调用。

## 变更

### `crates/session/src/tools/bash.rs`
- `use super::bg::{handoff, BgState}` -> `use super::bg::{register, unregister, BgState}`。
- 删除 `BASH_MAX_TIMEOUT_SECS` 常量、编译期守卫、`resolve_timeout_secs`（不再有自超时）。
- `execute()` 中 spawn 后 `register(pid, pgid, ctx.session_id)`；`child.wait().await?`（无超时）后 `unregister(pid)`。
- 前台正常完成：`code == 0` 时不再附退出码（有输出直接返回 streams，无输出返回 `(no output)`）；仅 `code != 0` 才追加 `\n[exit code: N]`。

### `crates/session/src/tools/bg.rs`
- 新增 `register` / `unregister` / `stop` / `list` 公共 API（全局注册表 `HashMap<u32, BgEntry>`）。
- `output_path`：`/tmp/opencode_bg_` -> `/tmp/opencoder_bg_`。
- 后台 supervisor（legacy `handoff`）：仅 `code != 0` 时才向输出文件 append `[exit code: N]`；成功不写入。
- 测试：`register_unregister_roundtrip`、`stop_kills_registered_process`（`#[tokio::test]`，串行化 `test_registry_mutex`）。

### `crates/session/src/runner/mod.rs`
- 删除 `pub(crate) use execute::DEFAULT_TOOL_TIMEOUT`（bash 不再引用，clippy dead-code）。

### `crates/session/src/runner/execute.rs`
- 测试：`match` 单分支 -> `if let`（clippy match_single_binding）。

### `crates/session/tests/tools_contract.rs`
- 旧 handoff 测试（`bash_tool_hands_off_on_timeout`、`bash_tool_output_file_captures_output_on_timeout`）替换为 register/unregister 集成测试（`bash_tool_runs_long_command_without_handoff`、`bash_tool_registered_and_stoppable`）。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| 成功输出不含退出码 | `bash_normal_completion` | `crates/session/src/tools/bash.rs`（unit） |
| 失败输出含退出码 + is_error | `bash_failure_appends_exit_code` | `crates/session/src/tools/bash.rs`（unit） |
| 长命令前台完成无 handoff | `bash_long_command_completes_without_handoff` | `crates/session/src/tools/bash.rs`（unit） |
| 运行中注册、完成后注销 | `bash_registers_while_running_unregisters_after` | `crates/session/src/tools/bash.rs`（unit） |
| schema 不暴露 timeout | `parameters_schema_hides_timeout_from_model` | `crates/session/src/tools/bash.rs`（unit） |
| register/unregister 往返 | `register_unregister_roundtrip` | `crates/session/src/tools/bg.rs`（unit） |
| stop 杀注册进程组 | `stop_kills_registered_process` | `crates/session/src/tools/bg.rs`（unit） |
| 前台长命令不交接 | `bash_tool_runs_long_command_without_handoff` | `crates/session/tests/tools_contract.rs`（integration） |
| 运行中可注册可 /stop | `bash_tool_registered_and_stoppable` | `crates/session/tests/tools_contract.rs`（integration） |

## 附带：`task_type` 编译阻塞修复（验证前置）

`SessionMeta` 新增 `task_type: Option<String>` 字段（parent/subagent 区分），但多处构造点未同步更新，导致工作树无法编译。最小修复：

- `crates/store/src/lib.rs`：re-export `TASK_TYPE_SUBAGENT`。
- `crates/session/src/runner/subagent.rs`：子会话 `task_type: Some(TASK_TYPE_SUBAGENT.into())`。
- `crates/session/src/lib.rs`：顶层会话 `task_type: None`。
- 其余全字段 `SessionMeta {` 构造补 `task_type: None`。

回归：`cargo test --workspace` -> 1376 passed / 0 failed；`cargo clippy --workspace --all-targets -- -D warnings` -> clean；`cargo build --workspace` -> clean。
