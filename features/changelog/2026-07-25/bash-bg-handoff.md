Commit: (working-tree, pre-initial-commit)

# feat(session/bash): 超时命令 handoff 到后台，不再 SIGKILL 杀组

## 背景

bash 工具在命令超过 `timeout` 时的旧行为是 `kill(-pgid, SIGKILL)` 杀掉整个进程组，
然后尽力回收部分输出。对长时间构建/测试/迁移这类「合法但慢」的命令，这意味着：

- 整棵进程树被强杀，已下载/编译的中间产物可能损坏，需要从头重来。
- 模型只拿到超时前那一瞬的输出，无法判断「真卡死」还是「只是慢」。
- 没有事后查看完整输出的途径。

更好的做法是**不杀**：把命令移交到独立后台 supervisor 继续跑，输出实时引流到文件，
让模型拿到 PID + 文件路径，命令结束后自动清理整组。

## 变更

### `crates/session/src/tools/bg.rs`（新增，218 行）
全局后台进程注册表 + handoff supervisor：
- `registry()`：`OnceLock<Mutex<HashMap<u32, BgEntry>>>`，键为 child pid。
- `BgState`：前台/后台共享的捕获状态。前台阶段（`file == None`）只缓冲到
  `stdout_buf`/`stderr_buf`；`handoff` 后激活 file 模式，`push_*` 同时追加到文件，
  保证后台输出文件持续 live。
- `output_path(pid)` -> `/tmp/opencode_bg_{pid}.output`。
- `handoff(pid, pgid, session_id, child, stdout_task, stderr_task, state)`：
  截断打开输出文件 -> 冲刷已缓冲的 stdout/stderr（stderr 前加 `[stderr]` 分隔）->
  激活 file 模式 -> 注册 pid -> spawn 独立 supervisor task。supervisor 持有 `child`
  （`kill_on_drop` 不会在工具 future 丢弃时提前杀进程），`child.wait()` 等待自然退出后：
  ① `kill(-pgid, SIGKILL)` 杀整组（关 pipe 写端，让 drain task resolve）
  ② 有界等 2s drain stdout/stderr task ③ 向文件追加 `\n[exit code: N]` ④ 移除注册项。
- `cleanup_all()`：drain 整个注册表，逐个 `kill(-pgid, SIGKILL)` + 删 temp 文件。
  程序退出时调用。

### `crates/session/src/tools/bash.rs`（改）
- 删除旧 `drain_partial`（组杀后回收部分输出的逻辑，现已无组杀）。
- timeout 路径改为 `bg::handoff(...)` 后返回 `ToolOutput::ok`，消息含 PID + output 路径
  （"moved to background ... Cleaned up automatically when it exits."）。
- 正常完成路径：用共享 `Arc<Mutex<BgState>>` + 增量推流（8KiB chunk）替代旧
  `read_to_end`，drain task 在 EOF resolve，语义等价但可在 handoff 时无缝切到文件。
- `setsid` / 进程组逻辑不变（仍 load-bearing：pid==pgid，退出后组杀一击尽杀后代树）。

### `crates/session/src/tools/mod.rs`（+1 行）
`pub mod bg;`

### 关闭钩子
- `src/main.rs`：进程退出前 `opencoder_session::tools::bg::cleanup_all()`。
- `crates/cli/src/run.rs`：第二次 Ctrl-C 强退时 `cleanup_all()`。
- 根 `Cargo.toml`：`+opencoder-session.workspace = true`（使二进制能调 cleanup_all）。

## 测试清单（rules/02-regression-gate）

新增/改写 6 个测试：

| 测试 | 位置 | 层级 | 断言要点 |
| --- | --- | --- | --- |
| `output_path_format` | bg.rs | unit | `/tmp/opencode_bg_{pid}.output` 格式 |
| `bg_state_push_buffers_when_no_file` | bg.rs | unit | file==None 时纯缓冲、不写文件 |
| `bash_normal_completion` | bash.rs | 集成 | 正常完成：stdout+stderr+exit code 正确 |
| `bash_handoff_on_timeout` | bash.rs | 集成 | 超时->handoff、消息含 "moved to background" + output 路径、PID 可解析、文件收到 `[exit code:]` |
| `bash_tool_hands_off_on_timeout` | tools_contract.rs | 集成 | 超时后孙进程 heartbeat 持续增长（证明未被杀）+ 文件收 `[exit code:]` |
| `bash_tool_output_file_captures_output_on_timeout` | tools_contract.rs | 集成 | 超时前打印的 marker 进了文件 + `[exit code: 0]` |

> `tools_contract.rs` 的两个旧测试为 **1:1 重命名**（非删除，无净增删）：
> `bash_tool_kills_process_group_on_timeout` -> `bash_tool_hands_off_on_timeout`；
> `bash_tool_returns_partial_output_on_timeout` -> `bash_tool_output_file_captures_output_on_timeout`。

回归（每条均为本次新鲜执行，非复用陈旧结果）：
- `cargo test -p opencoder-session` -> **268 passed; 0 failed**（29 个测试二进制求和）。
- `cargo test -p opencoder-cli` -> **48 passed; 0 failed**（4 个测试二进制）。
- `cargo test --workspace` -> **981 passed; 0 failed**（连续两次新鲜运行均 0 失败）。
- `cargo clippy -p opencoder-session --all-targets` -> 零警告（本功能 crate 净）。
- `cargo clippy --workspace --all-targets -- -D warnings` -> **零错误**。
- `cargo build --workspace` -> 全绿。

> 关于 `cargo test --workspace` 的如实说明：workspace 含范围外 model-switch /
> tui WIP。其 skill-token 测试存在已知 flake —— `app_helpers_tests` 的点击处理路径
> 在未持锁的情况下调用 `sys_tokens_for`，与 `with_home` 测试改写 `HOME` 竞争，
> 导致 `sys_tokens_for` 内两次 `home_dir()` 读取错位、token 估算偶发下溢为 0。
> 该竞态**非本功能引入**；低负载 / 孤立运行稳定全绿，重负载并行下偶发 5 例失败。
> 本次同时顺手修复 2 处范围外 lint（均非本功能代码）：cli `run.rs`
> `field_reassign_with_default`、tui `image_render.rs` `if_same_then_else`。

## 风险与对齐
- 时序依赖：3 个 handoff 测试含真实进程等待（3-4s sleep + 6-8s 轮询 deadline，
  余量 >= 2x）；handoff 行为本就需要真实验证，无法 mock。
- 退出后组杀仍依赖 `setsid`（pid==pgid）——与旧逻辑同一 invariant，未改变。
- 正常完成路径语义等价：增量 drain 替代 `read_to_end`，EOF resolve 行为一致。
- 纯函数式，无 `class`，符合仓库规则。bg.rs 218 行、bash.rs 296 行，均低于限制。
