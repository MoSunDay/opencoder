# plan 模式 bash_guard 修两处过度拦截（子 shell/花括号内 `2>&1`、`tee`）

## 背景

plan agent 的 bash 写拦截（`bash_guard::classify`）存在两处**过度拦截**（false positive），把本应只读的命令误判为写命令并拒绝，削弱 plan agent 的探查能力：

1. **子 shell / 花括号组内 `2>&1` 被误判为写**。`read_redirect_target` 读重定向目标时未识别 fd-merge 形式（`&N`），把 `(echo hi 2>&1)` 里的目标读成 `&1)`（一直吃到 `)`），目标里出现非空白字符即被当作"重定向到文件"→ 判写拦截。同理 `>/dev/null)`、`{ ls 2>&1; }` 等所有"重定向紧贴分组闭合符"的形式都中招。plan agent 因此无法运行 `(make 2>&1)`、`(go build ./... 2>&1)` 这类常见只读探查。

2. **`tee` 被无条件拦截**。`tee` 在 `MUTATING_COMMANDS` 常量列表里硬编码为写命令。但 `tee /dev/null`（丢弃输出，等价纯读）和裸 `tee`（只写 stdout，无文件副作用）都是只读；只有 `tee <真实文件>` 才是写。无条件拦截导致 `make 2>&1 | tee /dev/null`、`... | tee`（无参）这类只读管道被拒。

两者都是**严格放宽**（更多命令判为 ReadOnly），不影响真写拦截——文件重定向（`> file`）、`tee file` 仍被拦截。

## 变更

### `crates/session/src/bash_guard.rs`

- **`read_redirect_target`**：新增 fd-merge 分支——遇到 `&` 时只吃掉 `&` 及其后的 ASCII 数字（`2>&1` → 目标 `&1`），随后立即返回，不再吞掉紧跟的 shell 元字符。path 形式分支的终止符集合扩展：除原有 ` `、`\t`、`;`、`|`、`&&` 外，新增 `)`、`}`、`]`、`#`，使重定向紧贴分组闭合符时干净终止（`>/dev/null)` → 目标 `/dev/null`）。
- **`MUTATING_COMMANDS`**：移除硬编码的 `tee`。
- **`classify_segment`**：新增 `tee` 条件分支——仅当存在**非 flag 且 ≠ `/dev/null`** 的文件参数时判写（返回 `tee (writes to file)`）；否则（`tee /dev/null`、裸 `tee`、仅 flag 参数）判 ReadOnly。该判断位于 `cmd_base == "tee"`，先于 git 写检查。
- 公共 API `classify()` 签名与返回类型 `BashVerdict` 不变；变更为私有 `read_redirect_target` + `classify_segment` 一个分支 + 一个常量的行为放宽。

### `crates/session/tests/bash_guard_plan_mode.rs`

新增两条端到端集成测试，经真实 runner（`MockChatClient` + `tempdir`）验证 plan 模式下放行：

- `plan_mode_allows_subshell_fd_merge`：plan agent 执行 `(make 2>&1)` 不被拦截（`ToolEnd.is_error == false`）。
- `plan_mode_allows_tee_to_devnull`：plan agent 执行 `make | tee /dev/null` 不被拦截。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| fd-merge（`2>&1`/`1>&2`/`>&2`）紧贴 `)`/`}`/`]` 判只读 | `fd_merge_before_shell_metachars_allowed` | `crates/session/src/bash_guard.rs` |
| 子 shell/花括号组整体只读 | `subshell_and_brace_group_read_only` | `crates/session/src/bash_guard.rs` |
| 真实文件重定向紧贴 `)`/`}` 仍被拦截（防过度放宽） | `real_file_redirect_before_metachar_still_blocked` | `crates/session/src/bash_guard.rs` |
| `tee /dev/null` / 裸 `tee` / 仅 flag 判只读 | `tee_to_devnull_or_bare_allowed` | `crates/session/src/bash_guard.rs` |
| `tee <真实文件>` 判写拦截 | `tee_to_real_file_blocked` | `crates/session/src/bash_guard.rs` |
| plan 模式下 `(make 2>&1)` 经 runner 放行 | `plan_mode_allows_subshell_fd_merge` | `crates/session/tests/bash_guard_plan_mode.rs` |
| plan 模式下 `make \| tee /dev/null` 经 runner 放行 | `plan_mode_allows_tee_to_devnull` | `crates/session/tests/bash_guard_plan_mode.rs` |

- 新增 5 unit + 2 integration = 7 个测试，全部针对 `BashVerdict` / `ToolEnd.is_error` 等可观测输出断言，含正常路径与边界（真实文件紧贴元字符仍被拦截）。
- 全量回归（当次实跑）：`cargo test -p opencoder-session --no-fail-fast` → **158 passed / 0 failed / 0 ignored**（43 lib + 115 integration 跨 19 个集成 binary，含 6/6 `bash_guard_plan_mode`）。
- clippy：`cargo clippy -p opencoder-session --all-targets -- -D warnings` → 零警告。

## Gate（当次实跑取证）

| 项 | 结果 |
|----|------|
| `cargo test -p opencoder-session --no-fail-fast` | 158 passed / 0 failed / 0 ignored（lib 43 + 集成 115） |
| `cargo clippy -p opencoder-session --all-targets -- -D warnings` | 零警告 |
| `cargo build -p opencoder-session` | 零错误 |

> **范围说明**：本变更仅触及 session crate 的 `bash_guard` 模块（私有 fn + 常量，公共 `classify` 签名不变）。`cargo test --workspace` / `cargo build --workspace` 当前因**范围外、未提交、与本变更无关的 WIP**（`handoff_*` feature 跨 store/cli/web/tui 构造 `SessionMeta{}` 字面量缺字段）而 RED，非本变更引入。本变更在 session crate 范围内独立全绿；提交时仅暂存本 fix 的 2 个源文件 + 本 changelog，不混入 28 个范围外脏文件。
