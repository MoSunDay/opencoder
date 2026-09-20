Commit: 444e6b0e3aaaff7914d0ea4889f4f84818abebbd

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接。

## 索引
- `src/runtime.rs`、`src/runtime/` — 静态/动态共享四名额轮询调度、原子展开清单、按实例恢复、同组失败取消并收齐退出
- `src/exec/` — wasm（wasmtime WASI）与 agent 步执行；agent 步产出经 `extract_output_json_from` 三级提取：```json 围栏 → 尾部裸 JSON 兜底（`extract_tail_bare_json`，string-aware 括号平衡、取最后一个可解析顶层对象；坏围栏时从围栏体之后扫描）→ 整段解析
- `src/exec/agent_runc.rs` — agent 步容器沙箱分支：`dag.agent_sandbox="runc"` 时整段 session 移入容器执行（`BundleSpec` argv=Direct、`/usr/bin/agent-step-runner` 入口），产物回写 step 目录；与 host 共用 `exec/how_copy.rs` 的冻结 Agent 和本地 how 副本
- `examples/agent-step-runner.rs` — 容器内 session runner：读 step env（PROMPT/STEP_DIR/SESSION_ID/AGENT/HOW_APPEND）、跑 `opencoder_session::run`、写 transcript.txt/output.json/session.json（running→done/error），退出码 0/1/2
- `examples/agent-session-runner.rs` — 容器内多轮 agent session runner（`run_mode: agent` 自定义 agent 会话，host 以 `ArgvStyle::Direct` 拉起 `/usr/bin/agent-session-runner`，每轮一进程）：启动即 `set_var("OPENCODER_AGENTS_DIR")` 钉住只读 agents pool（resolve_agent/skill/tools/memory 全走 pool）；从 `<step_dir>/messages.json` 续接历史，跑一轮 `opencoder_session::run`，逐事件追加 `events.ndjson`（`{"kind": sse_kind, "payload": sse_data}` 一行一事件，host 用 `SessionEvent::from_sse` 还原并 tail），终局写全量 messages.json/transcript.txt/output.json/session.json；退出码 0/1/2
- `src/exec/wasm/host_imports*` — 模块名 `opencoder` 的 host imports：`opencoder_run_op(op_id,args)->exit`（`dag.ops` 白名单命令，进程组整树击杀、`<step>/ops/<op>.log` 证据、256KiB 截尾、默认 600s）与 `opencoder_http_probe(url,expect,timeout_ms,retries)->HTTP码|-1..-4`；未注册 op fail-closed trap；`sandbox: runc` 不注入；`crates/dag-review-tools` 为配套 wasm 模块 crate
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与批量上报
- `src/sandbox/` — OCI bundle/rootfs；`BundleSpec.knowledge`（`dag.knowledge_root` → `/workspace/knowledge` 只读 bind，wasm argv 附 `--dir`；fail-closed 校验+预建挂载点）、`BundleSpec.agents`（host 路径 → `/workspace/agent` 只读 bind：pinned agents pool（卡片+四共享池），DAG Agent 步和 agent-session 负载均挂载冻结资源池；同样预建挂载点，不给 wasm argv 加 `--dir`）与 `argv: ArgvStyle`（WasmModule|Direct）；in-process 沙箱以 `FsPerms::ReadOnly` preopen 同路径，`OPENCODER_KNOWLEDGE_DIR` 契约 env；`scripts/prepare-dag-rootfs.sh` 制备 rootfs（wasmtime + agent-step-runner + agent-session-runner + ldd 镜像）

- `src/exec/how_copy.rs` — 冻结原始 Agent/how；每次执行副本追加公共 how_append 与实例文本，Host/runc 共用且不回写资源。
- `src/exec/runc_events.rs` — 容器 Agent 事件按实例子会话导入现有事件存储。
- 实例 API、输入例子和恢复契约见 [动态步骤说明](../../docs/dag-dynamic.md)。

## 边界
- 执行只发生在 claiming 节点；runc fail-closed，不回落 in_process。
