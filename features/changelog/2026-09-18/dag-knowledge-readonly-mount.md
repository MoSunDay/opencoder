Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# M0：DAG 知识库只读挂载（`dag.knowledge_root`）

## 背景

## 变更

- `crates/core/src/config/runtime/dag.rs`：`DagConfig` 新增
  `knowledge_root: Option<PathBuf>`（只读知识根，绝对路径）与
  `agent_sandbox: AgentSandbox`（`host` 默认 / `runc`，节点级开关）；
  serde 默认与 merge（`config/merge.rs`）对齐——partial 覆盖不泄漏。
- `crates/dag-runtime/src/sandbox/oci.rs`：`BundleSpec` 新增
  `knowledge: Option<KnowledgeMount>` 与 `argv: ArgvStyle`（WasmModule 默认 /
  Direct 原样 argv，供 agent-step-runner 这类 rootfs 内建二进制）；mounts
  追加 `{"destination":"/workspace/knowledge","options":["ro","rbind"]}`，
  wasm argv 追加 `--dir=/workspace/knowledge`；`write_bundle` 对 knowledge 根
  fail-closed（缺失/符号链接/相对路径即 bail）并在私有 rootfs 预建挂载点。
- `crates/dag-runtime/src/exec/`：`StepCtx.knowledge_root`（runtime 从
  `deps.config.dag.knowledge_root` 填充）；`step_env` 增设
  `OPENCODER_KNOWLEDGE_DIR`；in-process 沙箱以 `FsPerms::ReadOnly` preopen
  同一 guest 路径；宿主 agent 步 prompt 尾注知识库路径与
  `git --no-optional-locks` 指引，工具 env 透传 `GIT_OPTIONAL_LOCKS=0`。

## 测试

- `opencoder-core --lib`：dag 块 serde/merge 新字段（含 partial 不泄漏）。
- `opencoder-dag-runtime --lib sandbox::`：knowledge 有/无两形态的
  config/mountpoint/argv、缺失与符号链接 fail-closed、Direct argv 透传。
- 进程级：`tests/dag_e2e/agent_runc.rs`（知识库 mtime/size 快照零写入断言 +
  bundle config ro 挂载断言）。
