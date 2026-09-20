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

## 验证

- 原生 `codex-cli 0.153.2` 经安装脚本放入只读 rootfs，在真实 runc 中启动成功；使用临时凭证目录和通过 `localhost` 域名访问的本地 Responses 模拟服务完成模型请求及一次 Shell/Git 工具调用，收到 `thread.started`、`command_execution`、`agent_message` 和 `turn.completed`，退出码 0。未调用真实账户模型服务。
- 超时验证分别覆盖包含镜像准备阶段的 DAG 步骤截止时间，以及已经准备好的真实容器的运行超时和进程回收；运行中 Codex 的取消通过独立场景验证。测试等待窗口为 180 秒，产品超时语义不变。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，零警告，包含最终测试修正。输出：`/tmp/opencoder-runc-codex-clippy-complete.log`。
- `cargo build --workspace`：通过。输出：`/tmp/opencoder-runc-codex-build.log`。
- 主工作区 `cargo test -p opencoder-worker --test dag_codex_runc -- --nocapture`：`1 passed; 0 failed`，真实 Server → Worker → runc 全链路及最终同步超时断言通过。输出：`/tmp/opencoder-runc-codex-main-e2e-synchronized.log`。
- 全量测试结果待执行完成后补齐。

本次只完成代码接入，不包含发布或生产 rootfs 替换。
