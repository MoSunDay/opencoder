# bash 超时拆分：模型可见显示值 120s / 实际强制值 130s

## 背景

`BashTool` 的前台超时曾用一个常量 `BASH_TIMEOUT_SECS=130` 同时承担两件事：
1. `tokio::time::timeout` 的真实强制截止（130 s）；
2. 超时后返回给模型的 handoff/killed 消息文案里的秒数（也显示 130 s）。

问题：消息显示 130 s，而 skill 文案（`do-and-done/SKILL.md`）写的是
"≥120s"，两处不一致；且缺少一个"在真实硬截止前把命令当作已超时"的缓冲语义。

期望语义：**文案 = 120s，实际 = 130s（+10 s buffer）**。即模型看到的数字
比真实强制值低 ~10 s，确保模型在硬截止真正触发前就被引导去把长任务后台化。

## 变更

**`crates/session/src/tools/bash.rs`**（单一改动文件）

- 新增常量 `BASH_TIMEOUT_DISPLAY_SECS`：生产 `120` / 测试 `1`，仅用于超时消息文案。
- 保留 `BASH_TIMEOUT_SECS`：生产 `130` / 测试 `1`，仍是 `tokio::time::timeout` 的真实截止值（不动）。
- 新增编译期不变量 `#[cfg(not(test))] const _: () = assert!(BASH_TIMEOUT_DISPLAY_SECS < BASH_TIMEOUT_SECS);`
  固化"显示值 < 真实值"的 buffer 关系——防止后人把两者"顺手纠正"成相等。
  注意：该 assert 仅在非 `cfg(test)` 编译时检查，故 `cargo test` 看不到它，必须靠
  `cargo build` 才能触发（见下方 Gate）。
- 两处超时消息（unix handoff、非 unix killed）把 `{BASH_TIMEOUT_SECS}s` 改为
  `{BASH_TIMEOUT_DISPLAY_SECS}s`。
- 更新 `BASH_TIMEOUT_SECS` 的 doc 注释，说明"显示 120 s / 实际 130 s（buffer）"的刻意差异。

设计决策（已确认）：**不**把超时值写进工具 `description()`。原因——超时触发时
handoff `ToolOutput` 本身就会作为工具结果返回给模型，模型届时就能读到"120s → 已后台化"，
无需预先在描述里声明，避免给模型一个可"协商/调高"的入口（与不暴露可调 `timeout` 参数的
安全护栏一致）。

## 测试覆盖

| 变更 | 测试 | 文件 | 层 |
|------|------|------|----|
| 文案走显示常量而非真实常量 | `bash_timeout_message_uses_display_constant` | `crates/session/src/tools/bash.rs` | unit |
| handoff marker/pid/路径不变（回归） | `bash_timeout_triggers_handoff` | 同上 | unit |
| 短命令不触发 handoff（回归） | `bash_short_command_completes_normally` / `bash_tool_runs_long_command_without_handoff` | 同上 | unit |
| schema 不暴露 timeout 参数（回归） | `parameters_schema_hides_timeout_from_model` | 同上 | unit |
| 120<130 编译期断言 | `const _: () = assert!(...)` | 同上 | compile（非 test 构建） |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test -p opencoder-session bash` | 45 passed / 0 failed（40 lib + 5 tools_contract，含新增 `bash_timeout_message_uses_display_constant`） |
| `cargo test -p opencoder-session --test tools_contract` | 18 passed / 0 failed |
| `cargo build -p opencoder-session` | Finished，零错误（`cfg(not(test))` 编译期断言 `120 < 130` 通过） |
| `cargo clippy -p opencoder-session --all-targets -- -D warnings` | 零警告 |

## Impact Surface
- **session/tools/bash**: 超时 handoff/killed 消息显示 120 s；真实强制仍是 130 s（+10 s buffer）。
- 模型在硬截止前 ~10 s 就会收到"已超时→后台化"信号。
