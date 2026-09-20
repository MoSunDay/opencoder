Commit: f2881a67b660651274d5f0e11e2709054c639677

# DAG runc Codex 接入

基线：`f2881a67`。此前 Codex DAG 仅能在 host 路径运行；runc 路径会要求原生 LLM API key，且容器没有 Codex CLI 和节点登录态。

## 变更

- 静态 Agent 步骤和动态 Agent 实例支持 `dag.agent_sandbox = "runc"` + `harness = "codex"`，纯 Codex DAG 不再要求 OpenCoder 原生 provider 凭证。
- 默认使用执行节点的 `CODEX_HOME` 或用户主目录下 `.codex`。显式 Harness/profile 配置优先；直接挂载原目录，保留认证刷新和会话文件的可写语义，不制作独立凭证副本。
- 私有启动配置位于 OCI bundle，独立于可查询 DAG 产物；容器接收固定的 profile、模型、认证槽位、推理强度和代理配置。步骤模型覆盖 profile 模型。
- 默认执行 rootfs 内 `/usr/bin/codex`；准入检查目录与可执行文件。缺失直接拒绝，认证和事件错误使步骤失败。容器执行、事件持久化、结构化输出、线程回执、超时和取消沿用 DAG 生命周期。
- `scripts/prepare-dag-rootfs.sh <rootfs> --codex <native-ELF>` 安装 CLI、Shell、Git、TLS 证书、动态库、NSS 解析模块、hosts 配置和宿主提供的 C.UTF-8 locale，不把节点凭证写进镜像。
- runc 提示词使用容器知识库路径；容器产物写入失败直接报错。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 节点默认凭证及显式目录优先级 | `credential_precedence_matches_host_process_inheritance` | `crates/dag-runtime/src/sandbox/codex/tests.rs` |
| profile、代理、模型和策略保留 | `guest_settings_keep_profile_and_credentials_private` | 同上 |
| rootfs 准入、私有挂载与权限、凭证不复制 | `profile_resolution_validates_guest_binary_and_builds_private_mounts` | 同上 |
| Server 下发到真实 runc；默认登录、依赖输出、动态实例/profile、事件与产物、认证失败、异常流、超时、取消及容器回收 | `server_dispatches_codex_in_runc_with_node_login_profiles_and_cancellation` | `crates/worker/tests/dag_codex_runc.rs` |
| 显式 `--target-dir` 与环境变量不同时，容器 runner 使用当前测试的目标目录与 profile | 同上，使用独立目标目录执行 | `crates/worker/tests/harness/runc_fixture.rs` |
| 真实 runc 启动、运行中取消/超时、标准输出溢出及容器回收 | `runc_step_smoke`、`cancellation_and_timeout_remove_running_containers`、`stdout_overflow_fails_and_removes_container` | `crates/dag-runtime/src/sandbox/runc.rs` |

## 验证

- 原生 `codex-cli 0.153.2` 经安装脚本放入只读 rootfs，在真实 runc 中启动成功；使用临时凭证目录和通过 `localhost` 域名访问的本地 Responses 模拟服务完成模型请求及一次 Shell/Git 工具调用，收到 `thread.started`、`command_execution`、`agent_message` 和 `turn.completed`，退出码 0。未调用真实账户模型服务。
- 超时验证分别覆盖包含镜像准备阶段的 DAG 步骤截止时间，以及已经准备好的真实容器的运行超时和进程回收；运行中 Codex 的取消通过独立场景验证。测试等待窗口为 180 秒，产品超时语义不变。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，零警告，包含最终测试修正。输出：`/tmp/opencoder-runc-codex-clippy-complete.log`。
- `cargo build --workspace`：通过。输出：`/tmp/opencoder-runc-codex-build.log`。
- 主工作区 `cargo test -p opencoder-worker --test dag_codex_runc -- --nocapture`：`1 passed; 0 failed`，真实 Server → Worker → runc 全链路及最终同步超时断言通过。输出：`/tmp/opencoder-runc-codex-main-e2e-synchronized.log`。
- 隔离基线全量 `cargo test --workspace`：退出码 0；按全部 `test result:` 行（含 doc-tests）汇总为 **5443 passed / 0 failed / 7 ignored**。7 项均为仓库既有的手动测试，新增测试全部执行。原始输出：`/tmp/opencoder-runc-codex-workspace-complete.log`；统计：`/tmp/opencoder-runc-codex-workspace-counts.json`。
- 旧版 glibc 的 hosts/DNS 解析及真实 CLI 域名请求通过；原生 CLI 的 Shell/Git 工具调用退出码 0。输出：`/tmp/opencoder-codex-native-dns-check.log`、`/tmp/opencoder-codex-native-tool-dns-check.log`。

## 补充回归

- 修正测试 fixture 内嵌 `cargo build` 的目标目录和 profile：与当前测试可执行文件保持一致，避免外层 `--target-dir` 覆盖环境变量后读取错误或旧 runner；容器测试构建关闭无需保留的调试符号。独立目标目录下完整 runc Codex 测试通过，输出：`/tmp/opencoder-runc-codex-custom-target-e2e.log`。
- 主工作区全量 `cargo test --workspace`：**5487 passed / 0 failed / 7 ignored**，退出码 0。同期完整 Clippy 和 workspace build 均通过。全量测试期间变化的 3 个 Worker 文件由后续 Worker 全套测试补验：**203 passed / 0 failed / 3 ignored**；再次完整 Clippy 通过。输出：`/tmp/opencoder-runc-codex-main-gate-evidence/`、`/tmp/opencoder-runc-codex-current-worker-tests.log`。
- 默认忽略的 3 项 runc 手动测试已在独立 rootfs 中显式执行：**3 passed / 0 failed**；包含真实 Wasm 容器、取消/超时和输出溢出回收。其余 4 项既有手动测试不属于本次 Codex 接入回归。输出：`/tmp/opencoder-runc-codex-manual-tests.log`。
- 后续合并 Worker 准入与调度锁改动后，在 `c047be99ab7228d060fdd9115881be0c6b1fdb39` 上再次运行 runc Codex 全链路及 `blocked_resource_read_does_not_starve_node_executor_or_lose_scoped_pool`：**2 passed / 0 failed**；Worker 全目标 Clippy 零警告，验证期间源码哈希无变化。输出：`/tmp/opencoder-runc-codex-post-merge-tests.log`、`/tmp/opencoder-runc-codex-post-merge-clippy.log`。
- 汇总及源码版本证据：`/tmp/opencoder-runc-codex-main-gate-evidence/validation-summary.json`。全量、Worker 和专项结果分别统计，不累加重复执行的用例。

本次只完成代码接入，不包含发布或生产 rootfs 替换。

相关说明：[接入与制备](../../../docs/registered-runners.md)、[Harness 能力](../../harness/index.md)、[DAG runtime](../../../agents/dag-runtime/index.md)。
