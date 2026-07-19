# bash 工具脱离控制终端：标准/错误输出不再泄漏进 TUI 输入区

## 背景

用户反馈：「处理 bash 的标准和错误输出，不要输出到 input 区域了！」——在 TUI 中运行 `bash` 工具时，命令的某些输出会渲染到 composer（输入框）区域，破坏界面。

根因调查：`BashTool` 已正确把 fd 0/1/2 分别设为 `Stdio::null()` / `Stdio::piped()` / `Stdio::piped()`，stdout/stderr **确实**进入管道并被 `truncate_output_with_error` 收回。问题在于子进程**仍继承控制终端**：任何对 `/dev/tty` 的直写（`sudo`/`ssh` 口令提示、进度条、登录 shell 的问候语、后台子进程等）会绕过管道，落在 alt screen 当前光标位置。而 `render.rs::place_cursor` 把光标定位在 `composer_area` 内——于是这些字节就「画」进了输入框。

先例：`crates/tui/src/input.rs:55-57` 已记录过同类「写 stderr 破坏 alt screen」问题；本次为同一缺陷类别的根因修复（在源头让子进程拿不到 tty，而非事后静默）。

## 变更

### `crates/session/src/tools/bash.rs`

- 在 `.kill_on_drop(true)` 之后、`cmd.output()` 之前，新增 `#[cfg(unix)]` 块：通过 `tokio::process::Command::pre_exec` 在 fork 后 exec 前调用 `libc::setsid()`，让子进程成为**新会话的 leader 且无控制终端**。此后对 `/dev/tty` 的 open/write 失败（ENXIO），所有输出被迫走已被管道捕获的 fd 1/2。
- 仅影响 unix；非 unix 平台该块被 cfg 剔除，原行为不变。

### `crates/session/Cargo.toml`

- `[dependencies]` 新增 `libc = "0.2"`（workspace 内 `crates/tui` 已用同版本，风格一致）。

### `crates/session/tests/tools_contract.rs`

- 导入追加 `bash::BashTool`，新增三条 `#[cfg(unix)]` 单测（见测试清单）。

## 设计取舍

- 选择 `setsid()` 而非 `setpgid`/重定向 `/dev/tty`：setsid 是 POSIX 异步信号安全、一步到位地「断掉控制终端」的标准做法；在 `pre_exec` 中调用是安全的（exec 前、fork 子内）。
- 未顺带改 `kill_on_drop` 的进程树清理（仅杀直系子进程）——属另一独立问题，本次最小化聚焦用户报告缺陷。
- 副作用：需要交互式 tty（如口令输入）的命令在非交互代理场景本就无法工作；现在会以显式 stderr 错误失败，而非静默破坏界面，更可观测。

## 测试清单

`cargo test -p opencoder-session --test tools_contract bash_`（全绿）：

1. `bash_tool_captures_stdout_via_pipe` —— stdout 经管道回到 `ToolOutput.content`。
2. `bash_tool_captures_stderr_via_pipe` —— stderr 经管道回到 `content` 且带 `[stderr]` 标记。
3. `bash_tool_detaches_controlling_terminal`（**回归核心**）——通过 `ps -o pid=,sid=` 断言子进程 `pid == sid`（即 setsid 生效、成为会话 leader）；无修复时 sid 为父会话 id，二者不等，断言失败。

回归验证：`cargo test -p opencoder-session` 全量通过（含 subagent / resume / steer / plan-guard 等既有套件）；`cargo build --workspace` 干净通过、无 warning。

## 后续加固（同一缺陷类别）：超时时按进程组清理子进程树

首轮修复（setsid）解决了「输出泄漏进输入区」，但 `execute()` 沿用的 `cmd.output()` + `kill_on_drop(true)` 模式留下另一个相关隐患：**超时时只杀直系 bash 子进程，孙进程（build/服务器/测试 runner/后台任务）作为孤儿存活**。既然 setsid 已让子进程成为会话 leader（`pgid == pid`），可一步到位地按整组回收。

### `crates/session/src/tools/bash.rs`（追加改动）

- 丢弃 `cmd.output()`，改为显式 `cmd.spawn()?` + 并发 drain 管道 + `tokio::time::timeout` race `child.wait()`。
- 取出 `child.id()` 作为 `pgid`（因 setsid，pgid==pid）。
- **并发 drain 管道**：`stdout`/`stderr` 各起一个 `tokio::spawn` 任务 `read_to_end`，与 `wait()` 并发——否则输出超过管道缓冲（~64 KiB）时进程阻塞在 write、`wait()` 永不返回、白白耗到 timeout。这是 `cmd.output()` 内部本就做的事，改用 spawn 后需显式重建。
- **超时分支**：`unsafe { libc::kill(-pgid, SIGKILL) }`（负 pid = 发往整个进程组）杀整棵子树，再 `child.wait().await` 回收直系子进程避免僵尸；孙进程被 reparent 给 init 回收。
- 保留 `kill_on_drop(true)` 作为 panic/提前返回路径的最后兜底（仅杀直系子进程）。
- 非 unix：`pgid` 与 kill 块均 `#[cfg(unix)]`，回退到旧的「仅直系子进程」语义（非真实目标平台）。
- 新增 import `use tokio::io::AsyncReadExt;`。

### `crates/session/tests/tools_contract.rs`（追加测试）

4. `bash_tool_kills_process_group_on_timeout`（**回归核心**）——后台一个心跳写入孙进程（非交互 `bash -lc` 无作业控制，孙进程与 bash 同组），给工具 1s timeout；超时后采样心跳文件两次，断言**不再增长**（孙进程已随组死亡）。

**变异验证**：临时把 `kill(-pgid, …)` 改成 `kill(pgid, …)`（仅杀直系子进程，模拟旧 bug），测试**确定性失败**（心跳 18→26 字节持续增长）；恢复后通过。证明该用例对本次修复敏感、可作回归闸门。

## 测试清单（最终）

`cargo test -p opencoder-session`（全绿，0 warning）：

1. `bash_tool_captures_stdout_via_pipe`
2. `bash_tool_captures_stderr_via_pipe`
3. `bash_tool_detaches_controlling_terminal`（setsid 回归）
4. `bash_tool_kills_process_group_on_timeout`（进程组 kill 回归，变异验证通过）

回归：既有 subagent/resume/steer/plan-guard/tools-contract 套件全数通过；`cargo build --workspace` 干净。

## 后续加固（二）：超时时返回已捕获的部分输出

前两轮修复了「输出泄漏进输入区」与「超时只杀直系子进程」。第三个相关问题：**超时时把已经 drain 到的部分输出全部丢弃**，只返回一行 `"command timed out after Ns"`。但超时恰恰是最需要诊断信息的场景——卡死的 build/test/服务器，它在挂起前打印的最后几行通常就是「为什么挂」的关键线索。丢掉它等于强迫 agent 盲目重试。

上一轮的进程组 kill 完成后，管道写端关闭、drain 任务以 EOF 收尾——所以部分输出**已经在我们手上**，只是早返回时没去取。

### `crates/session/src/tools/bash.rs`（追加改动）

- 抽出两个模块级私有 helper，去重并让成功/超时两条路径共享合并逻辑：
  - `fn merge_streams(stdout, stderr) -> String`——把两路合并，stderr 前缀 `[stderr]` 标记；全空返回空串（占位符由调用方决定）。
  - `async fn drain_partial(task: JoinHandle<Vec<u8>>) -> String`——bounded（500ms）等待 drain 任务；Join 失败/超时返回空串，绝不 wedge 工具。bounded 是为兜底「孙进程 setsid 逃逸出组 kill、仍持有管道写端」这种极端情形。
- **成功路径**改用 `merge_streams` + `format!` 拼 `[exit code: N]`，行为与输出格式与重构前逐字节一致（既有 4 条测试无需改动即通过）。
- **超时路径**：组 kill + `child.wait()` 回收僵尸之后，`drain_partial` 取回 stdout/stderr，`merge_streams` 合并，前缀 `"command timed out after Ns\n"`；全空时退回纯横幅。统一走 `truncate_output_with_error(.., true)`，与成功路径一致的截断语义。

### `crates/session/tests/tools_contract.rs`（追加测试）

5. `bash_tool_returns_partial_output_on_timeout`（**回归核心**）——`echo PARTIAL-MARKER-9f3a; sleep 30`，`timeout:1`；断言 `is_error`、内容含 `timed out` **且**含 `PARTIAL-MARKER-9f3a`（部分输出被保留）。

**变异验证**：临时把超时分支还原为旧的纯横幅 `ToolOutput::err(format!("command timed out after {N}s"))`（语法有效），测试**确定性失败**（`partial output discarded`，content 仅含横幅）；恢复后通过。证明该用例对本次行为修复敏感。

## 测试清单（最终，三轮累计）

`cargo test -p opencoder-session`（全绿，0 warning，`cargo clippy --tests` 干净）：

1. `bash_tool_captures_stdout_via_pipe`
2. `bash_tool_captures_stderr_via_pipe`
3. `bash_tool_detaches_controlling_terminal`（setsid 回归）
4. `bash_tool_kills_process_group_on_timeout`（进程组 kill 回归，变异验证通过）
5. `bash_tool_returns_partial_output_on_timeout`（部分输出回归，变异验证通过）

回归：既有 subagent/resume/steer/plan-guard/tools-contract 套件全数通过；`cargo build --workspace` 干净。
