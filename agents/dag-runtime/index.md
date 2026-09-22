Commit: d9b366a66dc7defa4281f484dd00b2a3e208c092

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接，执行只发生在 claiming 节点。

## 索引
- `src/runtime.rs`、`src/runtime/` — 调度、动态展开与恢复；静态/动态共享冻结 spec 的 `max_concurrency` 名额（缺省 4、范围 1–30）轮询调度、原子展开清单、按实例恢复、同组失败取消并收齐退出
- `src/exec/` — wasm 与 agent 步执行（含产出提取）
- `src/exec/wasm/in_process.rs` — 先设置 Store 的 epoch 截止点，再启动时钟线程，避免初始化阶段丢失取消；执行前已取消的令牌直接返回 Cancelled，不进入 guest。
- `src/exec/agent_runc.rs`、`src/sandbox/` — runc 沙箱（fail-closed）与 rootfs/挂载装配
- `src/sandbox/codex/` — 解析节点 Codex 登录目录与冻结 Harness/profile，校验 guest 可执行文件；原登录目录直接读写挂载，私有启动配置独立于 DAG 产物。`agent_runc` 保存线程回执、导入事件，并使用容器内知识库路径；纯 Codex 不创建原生模型请求。
- `src/exec/how_copy.rs` — 冻结原始 Agent/how；每次执行副本追加公共 how_append 与实例文本，Host/runc 共用且不回写资源。
- `src/exec/runc_events.rs` — 容器 Agent 事件按实例子会话导入现有事件存储；与 `how_copy` 同为容器/宿主共用执行件。
- `src/exec/wasm/host_imports*` — `opencoder` host imports
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与事件上报
- `examples/agent-step-runner.rs`、`examples/agent-session-runner.rs` — 容器内 session runner
- `examples/wasmtime-cli.rs`、`scripts/prepare-dag-rootfs.sh` — WASI 运行器与 rootfs 制备（wasmtime + agent-step-runner + agent-session-runner + ldd 镜像）

## 边界
- 执行只发生在 claiming 节点；runc fail-closed，不回落 in_process。
- 默认 host 路径由 `SessionState::new` 读取 Agent 卡的 `harness`；`codex` 沿用 session 的 Codex 子进程驱动和节点服务进程环境，未显式覆盖时使用节点的 `CODEX_HOME` 或该用户的 `~/.codex` 登录态。纯 Codex DAG 不需要原生模型 API Key，Server 不分发自身登录文件；显式 Harness/profile 环境仍优先。跨 Server/节点的凭证继承、依赖结果与认证失败契约由 [dag_codex 回归](../../crates/worker/tests/dag_codex.rs) 覆盖。
- runc Agent 按冻结 Harness 分派：原生执行器使用 LLM endpoint/API Key；Codex 使用 `sandbox/codex` 解析的 guest 可执行文件、私有配置及节点登录目录挂载，不要求原生 provider 凭证。容器知识路径由挂载合同确定，不能直接套用 host 路径。

## 相关
- [动态步骤说明](../../docs/dag-dynamic.md) — 实例 API、输入例子与恢复契约
- [Codex DAG 接入](../../docs/registered-runners.md) — host/runc 凭证、profile 与 rootfs 制备；安装脚本补齐 Shell、Git、TLS 和 NSS 解析依赖。

## 私有任务文件

`src/exec/private_files.rs` 只向 prompt 注入目录路径；host 使用执行目录，runc 将其只读挂载到 `/run/opencoder-task`（ro/nosuid/nodev），拒绝符号链接与重复挂载。`scripts/prepare-dag-rootfs.sh` 调用 `scripts/dag-rootfs/install-python.sh` 安装 Python 标准库和动态依赖，并执行 chroot 导入校验。
