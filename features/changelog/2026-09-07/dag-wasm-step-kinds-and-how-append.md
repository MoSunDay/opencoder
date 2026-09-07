Commit: c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6（开发基线；多轮工作树成果随本提交落地）

# DAG 步骤收敛为 agent/wasm 与 how_append 经验沉淀

## 问题与行为

DAG 步骤种类此前为 agent/python（内嵌 RustPython VM）。本迭代将其收敛为 **agent + wasm**（wasmtime WASI 命令模块），RustPython 整体移除——这是 LOCKED 线协议的**破坏性变更**：旧定义不做静默迁移，`opencoder_dag::decode_spec` 识别 python 步骤并返回专用错误「该定义使用已下线的 python 步骤，请改写为 wasm/agent 后重新保存」，所有解码入口（worker create、workloads/dag、control catalog save_dag）统一走该哨兵。

wasm 步骤契约：

- `command` = `"<module.wasm> [args...]"` 空白切分；模块路径相对且受困（拒绝绝对路径与 `..`）。查找顺序：run 上下文根（`<workflow_root>/<run_id>`，两种沙箱下 guest 均挂载 `/workspace/context`）→ 共享模块库 `<workflow_root>/_modules/`（worker 即 dag kind root；runc 模式下库内模块复制进 run 树供 guest 可见，已存在不覆盖）。模块库让控制面/运维可预先投放共享模块，也让测试获得无竞态的预投放位置（run 根在 Create 之前不存在，库目录与 journal 扫描互不干扰）。
- 环境契约：`OPENCODER_STEP_CONTEXT=/workspace/context/<step>/context.json`、`OPENCODER_STEP_DIR=/workspace/context/<step>/`、`OPENCODER_RUN_ID`；上游 context 以 `context.json` 文件交付（替代退役的 RustPython `context` 全局注入）。成功后可选解析 `<step>/output.json` 为结构化输出。
- `sandbox: in_process`（默认）= 内嵌 wasmtime，epoch deadline 超时 + 取消令牌跳变；`sandbox: runc` = 私有 OCI bundle + 容器内 `wasmtime run`，fail-closed（无 runc 即 Error，绝不静默回退）。`prepare-rootfs` 产出静态 wasmtime 运行时树。

agent 步骤新增 `how_append`（≤ `MAX_HOW_APPEND_BYTES`）：步骤 Done 后把 spec 声明的值追加到 agent 共享池的 `how.md`（池名 = agent 卡 `current.prompt` 引用的共享池，无卡则 agent 名/默认 `act`），快照全部现有 `*.md` + 追加行生成新 prompts 版本。传递链：`exec/agent.rs` 预置 `SessionState.env_passthrough` → `ToolContext.extra_env` → bash 工具 `.envs` 注入 `OPENCODER_HOW_APPEND`（session 运行时内的环境变异不回收，只认 spec 声明值）；追加失败仅 warn，不影响步骤结果。

附带修复：`write_context_json` 原先把 run_id 拼了两遍，现统一走 `artifacts::step_dir(&ctx.workflow_root, …)`。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| StepKind 解码（agent 字段/wasm command/迁移哨兵/how_append 上限） | `spec module` 19 用例 | `crates/dag/src/spec.rs` |
| wasm 执行：stdout/output.json/非零退出/epoch 超时/取消/模块解析（run 根优先、库回退、受困） | `exec::wasm::tests` 9 用例 | `crates/dag-runtime/src/exec/wasm/tests.rs` |
| how_append 环境对/池名/追加 | `how_append` 3 用例（OVERRIDE_LOCK 保护） | `crates/dag-runtime/src/exec/how_append.rs` |
| OCI bundle（wasm argv/env）与 rootfs 运行时树 | `write_bundle_writes_config_and_private_rootfs` 等 | `crates/dag-runtime/src/sandbox/{oci,rootfs}.rs` |
| 真实 runc 手动链（stage_module/HELLO/SPIN/OVERFLOW） | 3 个 manual 测试 | `crates/dag-runtime/src/sandbox/runc.rs` |
| worker 节点级 wasm 流程（工件/重启/取消/晚到 cancel/256MiB 工件流） | `dag_artifacts_and_checkpoints_survive_node_restart`、`dag_cancel_interrupts_wasm_step_and_releases_node_capacity`、`completed_execution_wins_a_late_cancel…`、`streams_256_mib_artifact…` | `crates/worker/tests/{workloads,artifact_stream,durable_lifecycle}.rs` |
| 全解码入口拒绝 python 步骤 | worker/control/web/e2e fixtures 全量换 wasm | `crates/{worker,control,web}` 对应测试 |

- `cargo check --workspace --all-targets` 干净；`cargo test --workspace --locked --no-fail-fast` 全绿（含此前一次 hub 超时疑似并发抖动复验）。
- SPA 450 用例绿、dist 无漂移（`check-spa-drift.sh`）。
