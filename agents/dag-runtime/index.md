Commit: 34f69db14b8c6818823c1eb01696131dabba7e23

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接。

## 索引
- `src/runtime.rs` — 步骤调度（并发上限、取消传播）
- `src/exec/` — wasm（wasmtime WASI）与 agent 步执行；agent 步产出经 `extract_output_json_from` 三级提取：```json 围栏 → 尾部裸 JSON 兜底（`extract_tail_bare_json`，string-aware 括号平衡、取最后一个可解析顶层对象；坏围栏时从围栏体之后扫描）→ 整段解析
- `src/exec/agent_runc.rs` — agent 步容器沙箱分支：`dag.agent_sandbox="runc"` 时整段 session 移入容器执行（`BundleSpec` argv=Direct、`/usr/bin/agent-step-runner` 入口），产物回写 step 目录；host 路径零变化
- `examples/agent-step-runner.rs` — 容器内 session runner：读 step env（PROMPT/STEP_DIR/SESSION_ID/AGENT/HOW_APPEND）、跑 `opencoder_session::run`、写 transcript.txt/output.json/session.json（running→done/error），退出码 0/1/2
- `src/exec/wasm/host_imports*` — 模块名 `opencoder` 的 host imports：`opencoder_run_op(op_id,args)->exit`（`dag.ops` 白名单命令，进程组整树击杀、`<step>/ops/<op>.log` 证据、256KiB 截尾、默认 600s）与 `opencoder_http_probe(url,expect,timeout_ms,retries)->HTTP码|-1..-4`；未注册 op fail-closed trap；`sandbox: runc` 不注入；`crates/dag-review-tools` 为配套 wasm 模块 crate
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与批量上报
- `src/sandbox/` — OCI bundle/rootfs；`BundleSpec.knowledge`（`dag.knowledge_root` → `/workspace/knowledge` 只读 bind，wasm argv 附 `--dir`；fail-closed 校验+预建挂载点）与 `argv: ArgvStyle`（WasmModule|Direct）；in-process 沙箱以 `FsPerms::ReadOnly` preopen 同路径，`OPENCODER_KNOWLEDGE_DIR` 契约 env；`scripts/prepare-dag-rootfs.sh` 制备 rootfs（wasmtime + agent-step-runner + ldd 镜像）

## 边界
- 执行只发生在 claiming 节点；runc fail-closed，不回落 in_process。
