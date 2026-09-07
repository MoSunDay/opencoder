Commit: fdde836e5471a2d9d4f5f5a24bad0016ff7c2274（开发基线；两轮工作树成果随本提交落地）

# macOS 编译支持：session/process 平台门控与发布链路去 Linux 写死

## 问题与行为

`opencoder`（CLI，真实包名带 r）在 macOS 编译直接失败：依赖链 `opencoder → cli → todos → session` 中 `crates/session/src/process/` 有 5 处 Linux-only libc 符号（`prctl(PR_SET_PDEATHSIG)`、`pipe2`、`SYS_pidfd_open`、`SYS_pidfd_send_signal`、`PR_SET_CHILD_SUBREAPER`）完全无 `#[cfg]` 门控。`opencoder-server` 代码层面无硬阻塞，但发布链路写死 Linux（强制连带编 agent + GNU `sha256sum`），README badge 虚报 `windows`，且无任何 macOS CI。

- **session/process 拆为门面 + 双后端**：`mod.rs` 保留可移植 API（`configure_owned_command`、`wait_owned_child`、`supervisor_args`）并按 `#[cfg(target_os)]` 选后端；Linux 粘合（`build`/`SpawnLease`/`OwnedSupervisor`/`configure_supervisor_binary`/`LEASE_FD`，原逻辑逐字搬移）落 `owned.rs`；非 Linux 走新增 `fallback.rs` fail-closed stub——`command()/runc_command() → Ok(None)`（调用方既有 direct-Command fallback 分支闭环，零调用方改动）、tracker → `0/Ok(())`、配置与入口 API → Err（"process supervision is Linux-only"）；`SignalTarget/SpawnLease/OwnedSupervisor/RuncCleanup` 类型在两平台均可编译，`bg.rs`、`process_group.rs`、dag-runtime、worker、agent 的类型级引用零改动。原 mod.rs 的 pidfd 测试改 `#[cfg(all(test, target_os = "linux"))]`。
- **worker/resources.rs**：`check_mount` 的 `/proc/self/mountinfo` 校验体加 `#[cfg(target_os = "linux")]`，非 Linux fail-closed bail。
- **tests/tui_exit_restore_e2e.rs**：util-linux `script -q -f -c` 语法在 BSD script 下不可用，新增 `pty_harness_available()`（macOS 恒 false）替换 4 处工具探测 gate。
- **macOS 测试可用性**：macOS 无 `setsid(1)`，`bash/background_tests.rs`（5 处）与 `tests/bg_kill_all.rs` 加运行时 skip guard。
- **scripts/platform/release/build.sh**：按 `uname -s` 分支（Linux 三元 / Darwin `opencoder opencode-server` / 其它 exit 6）；`sha256sum` 依赖改为 `sum_files/sum_check` 双工具适配（无则 `shasum -a 256`）；SPA digest 改纯 python3 计算（脚本内自洽）；manifest 与 SHA256SUMS 由 binaries 数组生成，Linux argv 与原脚本逐 token 一致。
- **scripts/platform/install_bundle.py**：校验集合由 manifest 驱动（`bundle_names()`，非空 ⊆ 已知全集且必含 `opencoder`，bin 目录内容必须与 manifest 声明一致），不再写死"恰好三个"；launcher 按当前集合管理并在集合收缩时清理悬空符号链接；锁、原子切换、回滚语义不变。
- **README/README.en**：badge 改 `linux | macos`；补源码构建平台前置（macOS 需 `xcode-select --install` + `brew install cmake`；agent 仅 Linux；macOS 会话子进程直连运行）。
- **新增 `.github/workflows/platform.yml`**：macos-latest 上 `cargo check/clippy -p opencoder -p opencoder-server --all-targets` + `cargo test -p opencoder-session`，防再次引入无门控 Linux-only 符号。
- 范围外（另行立项）：`opencoder-agent` 的 macOS 适配（dag-runtime `exec/python/process.rs:177` 的 `PR_SET_PDEATHSIG` 与 runc 沙箱均为 Linux-only，属 agent 闭包）。注：调查简报中的 `opencode`/`opencode-server` 实际包名为 `opencoder`/`opencoder-server`。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| fallback 契约（command/runc_command→None、configure/supervisor_main/lease→Err、tracker 0/Ok） | 5 个契约测试（仅非 Linux 平台编译执行） | `crates/session/src/process/fallback.rs` |
| Linux 监管行为零变化 | `outer_timeout_never_force_kills_a_supervised_owner`（cfg linux）+ tracker/pidfd 原测试 | `crates/session/src/process/` |
| session 全量回归 | 834 passed / 0 failed | `cargo test -p opencoder-session` |
| TUI 恢复 e2e gate 不破坏 Linux | 4 passed | `tests/tui_exit_restore_e2e.rs` |
| setsid 缺失可跳过 | skip guard ×6（Linux 照常执行） | `background_tests.rs`、`tests/bg_kill_all.rs` |
| 两元 bundle 可安装、manifest 驱动拒绝、集合收缩清理 | 3 个新增测试（合计 Ran 8 OK） | `scripts/platform/test_install_bundle.py` |
| build.sh 语法与 Linux argv 形状 | `bash -n` + 展开核对 | `scripts/platform/release/build.sh` |
| CI 防回归 | platform.yml（macos check/clippy/test） | `.github/workflows/platform.yml` |

## 回归记录与已知边界

- `cargo fmt --check`：本任务全部改动文件 clean；工作树另有**并发任务在途改动**（web/store 等）存在两处 fmt diff 与 store clippy `derivable_impls`（后者已做零语义机械修复：手写 Default 改 derive）。
- `cargo clippy -p opencoder-session -p opencoder --all-targets -- -D warnings` 零警告。全 workspace 门禁被并发任务的 dag-runtime 半途迁移阻塞（其 Cargo.toml 已移除 rustpython-vm、代码未迁移，6–9 个编译错误，非本任务引入），故按 dag-runtime 闭包范围化；`cargo check -p opencoder-worker` 在并发破坏发生前已通过（含 resources.rs 改动），并以 rustfmt 解析复核。
- 脚本：`python3 -m unittest test_install_bundle` 8 tests OK；`test_data_archive` 因容器 Python 缺 `_sqlite3` 模块导入失败（环境限制，与本任务无关）。
- macOS 实机验证以新增 CI 为准；本机不可交叉 check session 全 crate（`ring` C 依赖需 macOS SDK），fallback.rs 已单独交叉编译到 aarch64-apple-darwin 验证。

## Fast-follow（评审后快修，同工作树）

评审提出两项中级发现，均分钟级修复，随本轮一并落地：

1. **`supervisor_args` 死代码门控**：`crates/session/src/process/mod.rs` 中该 `pub(crate)` 函数的唯一消费者（`supervisor.rs` 与门面测试）均为 Linux-gated，非 Linux 构建下触发 `dead_code` 警告（违背零警告基线，且 macOS 上 `clippy -p opencoder-session -D warnings` 即红）。修复：函数与其专属的 `anyhow::{Context, Result}`、`std::ffi::{OsStr, OsString}` import 一并加 `#[cfg(target_os = "linux")]`。
2. **CI paths 覆盖缺口**：`platform.yml` paths 原仅含 core/session/cli/tui/web/server 六目录，而 `opencoder`+`opencode-server` 实际编译闭包经 `cargo metadata` 实测为 14 个 crate（另含 store/llm/shellguard/todos/agents/brain/control/dag/node，多于评审列出的 4 个；web 反而不在闭包内）。按评审许可的宽口径修复：paths 放宽为 `crates/**`，结构性消除「闭包增长而 paths 不更新」的本事故复发通道；Linux-only crate（worker/agent/dag-runtime/project/team/web）改动仅产生一次冗余绿灯运行。

### 快修测试清单

| 验证项 | 结果 | 手段 |
| --- | --- | --- |
| 非 Linux 死代码警告修复前确实存在 | pre-fix 变体 darwin 交叉 check 复现 `function supervisor_args is never used` | 探针 crate（session process 门面 + fallback 逐字拷贝，`--target aarch64-apple-darwin`） |
| 修复后非 Linux 零警告 | check + clippy 双绿（`RUSTFLAGS=-D warnings`） | 同上探针 crate |
| Linux 行为零变化 | session 全量 834 passed / 0 failed（与快修前基线逐数一致） | `cargo test -p opencoder-session` |
| Linux lint 零回归 | fmt clean；clippy session + opencoder `--all-targets -D warnings` 零警告 | `cargo fmt/-p`、`cargo clippy` |
| workflow 语法与 paths 生效 | YAML 解析通过，push/PR paths 均为 `crates/**` 等 4 条 | `yaml.safe_load` 断言 |

评审 TODO 其余项处置：docs `opencode*` 名称一致性已核查（`docs/` 下无漂移，无需修改）；全 workspace 门禁仍被并发任务 dag-runtime 半途迁移阻塞（本轮实测仍有 9 个编译错误），维持按闭包范围化，待其落地后补跑。
