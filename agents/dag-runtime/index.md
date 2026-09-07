Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# dag-runtime — 节点侧 DAG 执行运行时

[worker](../worker/index.md) 通过 `Uplink::for_local_dag` 将事件和状态写入 Node Store。Server 只接收执行索引；平台的 wasm/runc 执行由 `opencoder-agent` 承担，`opencoder-server` 不链接该运行时。步骤只有两种：agent（真 session）与 wasm（wasmtime WASI 命令模块）。

## 执行与恢复

- `runtime.rs` 验证定义、并发调度至多 4 个步骤，传播取消令牌，持久化步骤输出并折叠终态。wasm 步骤用 epoch deadline 处理超时，取消令牌触发 epoch 跳变；外层不能提前丢弃清理 future。
- `checkpoint` 恢复 `meta.json` 为 done 且产物可读的步骤；先 fsync 输出，最后写 meta。恢复只由显式 resume 触发，已完成步骤不重跑。
- `step_io` 统一记录产物和事件；写入失败使步骤失败并阻断依赖。节点本地事件持久化错误由适配器锁存并返回。
- 旧 REST Uplink 保留兼容与测试用途；终态上报短退避重试一次，仍失败则向执行 owner 返回错误，不再只 warn 后报告正常返回。`dag_events` 批量队列按 8 条或 300ms 刷新，终态前等待关闭。

## Wasm 生命周期

`exec/wasm/mod.rs` 执行 WASI 命令模块（RustPython 已整体移除）。`command` = `"<module.wasm> [args...]"` 空白切分，模块路径相对且受困（拒绝绝对路径与 `..`）：先查 run 上下文根（`<workflow_root>/<run_id>`，两种沙箱下 guest 均挂 `/workspace/context`），再查共享模块库 `<workflow_root>/_modules/`（worker 的 dag kind root 或 project 本地 workflow root；runc 模式下库内模块复制进 run 树供 guest 可见，已存在则不覆盖）。上游 context 落盘为 `<run>/<step>/context.json`，路径经环境变量传递：`OPENCODER_STEP_CONTEXT` / `OPENCODER_STEP_DIR` / `OPENCODER_RUN_ID`。成功后可选解析同目录 `output.json` 为结构化输出，`output.txt`/`meta.json` 由运行时负责。

`_modules` 是保留名：`opencoder_dag::artifacts::validate_run_id` 拒绝恰好为 `_modules` 的 run id（否则与库目录冲突得到费解的 409/目录已存在错误）；worker 布局迁移（`worker/src/migration.rs`）把 `_modules` 与 `rootfs`/`bundles` 同列为 workflow 根下的非执行目录豁免，legacy 节点使用模块库后仍可完成迁移。

`sandbox: in_process`（默认）内嵌 wasmtime 引擎（epoch 超时 + 取消）；`sandbox: runc` 走私有 OCI bundle + 容器内 `wasmtime run`，fail-closed：节点无 runc 即 Error，绝不静默回退 in_process。`sandbox/runc` 使用独立 bundle 状态目录，取消/超时执行有界 force delete 并回收 launcher，清理错误返回调用方。`sandbox/rootfs` 为每个 bundle 复制独立运行时树（`usr/bin/wasmtime` 等）并复用到重试；复制在 blocking 工作线程执行，运行时以只读 root 挂载。容器 ID 是 run 与 step 的组合，允许两个合法的长 ID 组合。`sandbox/oci` 生成只读 rootfs 和可写 run bind；rootfs 必须为真实目录，不能是 symlink。`prepare-rootfs` 脚手架产出静态 wasmtime 树；`scripts/prepare-dag-rootfs.sh` 在此之上把 `examples/wasmtime-cli`（复用执行器同款 wasmtime/wasi crate 的最小 CLI）连同动态库装进 rootfs，配 `DAG_TEST_ROOTFS` 可实跑 `sandbox::runc` 的 3 个 manual 测试（已在本机 runc 实测通过）。

## how_append（agent 步骤经验沉淀）

spec 中 agent 步骤可声明 `how_append`：步骤 Done 后把该值追加到 agent 共享池的 `how.md`（池名 = agent 卡 `current.prompt` 引用的共享池，无卡则 agent 名/默认 act），生成新 prompts 版本（快照全部现有 `*.md` + 追加行，`"\n\n"` 连接、trim）。实现在 `exec/how_append.rs`，经 `SessionState.env_passthrough`（`ToolContext.extra_env` → bash `.envs`）注入 `OPENCODER_HOW_APPEND`，session 运行时内的环境变异不回收；失败仅 warn 不影响步骤结果。

## 验证入口

- wasm 正常输出、output.json、非零退出、超时 epoch、取消、模块解析（run 根优先/库回退/受困）：`exec/wasm/tests.rs`。
- how_append 环境对、池名解析与追加：`exec/how_append.rs`（OVERRIDE_LOCK 保护）。
- 状态上报故障、依赖阻断和取消 drain：`tests/run_loop.rs`。
- 真实 runc：显式执行 `sandbox::runc::tests` 的 manual 测试，并设置已有测试变量 `DAG_TEST_ROOTFS`；缺失前提会失败，不静默跳过。
