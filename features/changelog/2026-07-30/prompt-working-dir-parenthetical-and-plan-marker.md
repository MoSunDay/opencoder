# prompt：environment_block 工作目录行收为括注 + PLAN 模式只读标记

## 背景

`environment_block` 原以两行分别表达工作目录与「不得越界」约束：

- `- Working directory: {cwd}`
- `- Stay within the working directory: you may work in its subdirectories, but do not access or modify anything outside it.`

第二行语义与第一行的目录信息高度耦合，拆成两行浪费 token。此外，PLAN 模式下缺少明确的只读提示，模型可能在调查阶段尝试写文件（虽然 `bash_guard` 会在 plan 模式拦截写操作，但系统提示中无对应文字暗示）。

## 改动

`crates/session/src/prompt.rs`：

1. `environment_block` 签名由 `(working_dir: &Path)` 改为 `(working_dir: &Path, kind: AgentKind)`，调用方 `build_system` 传入 `agent.kind`。
2. 工作目录与越界约束合并为单行括注：

   > `- Working directory: {cwd} (may enter subdirectories, do not go outside it)`

3. 当 `kind == AgentKind::Plan` 时追加只读标记（ACT 模式省略以节省 token）：

   > `- IN_PLAN_MODE: read-only — do not edit/write files; mutating bash is intercepted. Investigate read-only and output a plan only.`

依赖项 `AgentKind` 与 `Agent.kind` 均为既有类型（`crates/core/src/agent.rs`），无新增类型。`environment_block` 的全部调用点仅 `build_system`（本文件）与 `crates/session/tests/prompt.rs`，改动自洽。

## 影响

- 仅修改系统提示拼装，不触碰任何执行路径或工具行为。
- 文本为 ASCII，遵循「默认 ASCII」。
- PLAN 模式的只读拦截仍由 `bash_guard` 兜底，系统提示文字为补充暗示，非唯一防线。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| 工作目录行含 cwd、Platform、Date | `environment_block_contains_cwd_and_platform` | `crates/session/tests/prompt.rs` |
| 工作目录行含新括注措辞 | `environment_block_constrains_to_working_directory` | `crates/session/tests/prompt.rs` |
| PLAN 模式含 `IN_PLAN_MODE` 只读标记 | `environment_block_marks_plan_mode_readonly` | `crates/session/tests/prompt.rs` |
| ACT 模式省略 `IN_PLAN_MODE` 标记 | `environment_block_omits_plan_marker_in_act` | `crates/session/tests/prompt.rs` |
| `build_system` 端到端含 agent prompt 与 environment | `build_system_includes_agent_prompt_and_environment` | `crates/session/tests/prompt.rs` |

## 验证

在本工作树（HEAD `8d6aafa` + 本改动）上实跑：

- `cargo build --workspace` -> **Finished**（干净）。
- `cargo clippy --workspace --all-targets -- -D warnings` -> **0 warnings**。
- `cargo test -p opencoder-session --test prompt` -> **23 passed / 0 failed / 0 ignored**。
- `cargo test --workspace` -> **1396 passed / 0 failed / 0 ignored**。
