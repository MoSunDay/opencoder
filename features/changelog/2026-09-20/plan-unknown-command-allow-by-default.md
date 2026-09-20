# plan 模式 unknown 命令 allow-by-default

## 背景

- plan/sidecar 只读 bash 门经 `bash_guard.rs` 适配 `opencoder-shellguard`
  分类器。此前未注册命令（handler 注册表之外的二进制，如 `pip`、`apt`、
  `kill`、`sudo`、`stdbuf`、`exec`、`eval`）一律 `default_verdict` → `Ask`，
  在只读会话中被统一拦截，导致大量无害只读命令（`pip list`、`apt list`、
  `kill -9`）也无法执行。

## 变更

- `crates/shellguard/src/analyzer_dispatch.rs::default_verdict`：unknown
  命令的缺省判定从 `Ask` 改为 `Allow`，出处类型化为新增的
  `AllowReason::UnknownCommand`（`{cmd} (unknown command)`），保持可审计。
- 以下 fail-closed 面保持不变：不可解析输入（`unparseable command`）、
  `$()`/`<()` 动态展开、已注册写命令（rm/touch/git push/…）、写重定向
  （unknown 命令 + `> file` 仍按 most-restrictive 组合拦截）、
  已注册交互 shell（裸 `sh` 仍 Ask）、解释器 `-c` 负载分析。
- `crates/session/src/bash_guard.rs` 无代码改动：映射层把新 `Allow`
  （`writes_state=false`）原样放行，策略翻转全部发生在 shellguard 判定层。

## 语料翻转（compat corpus）

- `bash_guard_compat_tests.rs`：`compat_over_blocks_unknown_command_fail_close`
  重写为 `compat_unknown_commands_allow_by_default`（含 unknown+写重定向
  仍拦截的反向行）；`php -r`、`sudo bash -c 'rm x'` 翻转为放行。
- `bash_guard_compat_tests2.rs`：`kill -9`、`dd`、apt/pip/cargo/brew、
  `sudo …`、`exec/eval/source/.`、`ionice/stdbuf/setsid`（未注册无展开）、
  `env -u FOO`/`nice -n5`（旗标取值误判为命令名，旧靠 fail-closed 兜住）
  等行翻转为放行；`time`/`env -i`/`nice -n 5` 等已注册写命令仍拦截。
- `crates/shellguard/src/classify_in_tests.rs` 新增 4 个策略回归用例：
  unknown 放行 + `UnknownCommand` 出处、unknown+写重定向仍拦截、不可解析
  fail-closed、已注册写命令仍拦截。

## 涉及文件

- `crates/shellguard/src/analyzer_dispatch.rs`、`src/allow_reason.rs`、
  `src/lib.rs`（文档）、`src/handlers/mod.rs`（注释）、
  `src/classify_in_tests.rs`（新用例）。
- `crates/session/src/bash_guard_compat_tests.rs`、
  `bash_guard_compat_tests2.rs`（语料翻转）。
- `agents/shellguard/index.md`、`agents/session/index.md`（memory 同步）。

## 验证

- `cargo test -p opencoder-shellguard`：378 passed。
- `cargo test -p opencoder-session --lib`：456 passed（含翻转后语料）。
- `cargo test -p opencoder-session --test bash_guard_plan_mode --test
  plan_subagent_guard --test clear_context_bash_gate --test compound_cmd`：
  全绿。
- workspace 全量回归见同日 regression-gate 条目。
