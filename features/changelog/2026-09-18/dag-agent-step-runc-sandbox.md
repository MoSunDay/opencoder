# DAG agent 步 runc 沙箱化：session 整体移入容器执行

## 背景

- wasm 步的 runc 沙箱此前已交付；M0 补齐了知识库只读挂载地基
  （`dag.knowledge_root`/`agent_sandbox` 配置、OCI bundle RO bind、
  rootfs 制备脚本，见 `dag-knowledge-readonly-mount.md`），但 agent 步
  仍在宿主进程内跑 `opencoder_session`，与"节点执行面不可信负载必须沙箱化"的边界不符。
- 目标：`dag.agent_sandbox="runc"` 时 agent 步与 wasm 步同一裁决口径，
  M0 host 路径行为零变化。

## 变更

- 新增容器内 runner `crates/dag-runtime/examples/agent-step-runner.rs`
  （rootfs 内安装为 `/usr/bin/agent-step-runner`）：读 step env
  （`OPENCODER_STEP_PROMPT` 必填缺失退出码 2、STEP_DIR 缺省
  `/workspace/context/step`、SESSION_ID/STEP_AGENT 缺省回退、HOW_APPEND 与
  `GIT_OPTIONAL_LOCKS` 透传），`Config::load("/workspace")` 失败回退
  default 并告警，单线程 tokio 跑 `opencoder_session::run`，64KB push_tail
  累积 transcript（空则 `last_assistant_text` 兜底），写
  transcript.txt / output.json（复用 `extract_output_json_from`）/
  session.json（running→done|error）。
- 新增 `crates/dag-runtime/src/exec/agent_runc.rs`：`execute_agent_step`
  入口按 `dag.agent_sandbox==Runc` 分派（`exec/agent.rs`，host 路径不动，
  `step_agent_name/build_prompt/create_session_meta` 提为 pub(crate)）；
  runc 分支 fail-closed（`runc_available` 不过即 error outcome），host 侧
  仍写 session 元数据与 context.json/prompt.txt，BundleSpec
  `command=["/usr/bin/agent-step-runner"]`、argv=Direct、knowledge 只读挂载，
  env 注入 OPENAI_BASE_URL/OPENAI_API_KEY/OPENCODER_MODEL 等凭据，
  `run_step_streamed` 流式回传日志；退出 0 走
  `finish_from_output_json`，非 0/Err 仿 wasm 错误映射（cancel→Cancelled）。
- `scripts/prepare-dag-rootfs.sh` 扩展：构建并安装
  `wasmtime-cli` 与 `agent-step-runner` 两个 example，ldd 依赖镜像覆盖两二进制。
- `crates/core/src/config.rs`：re-export `AgentSandbox`（供 runtime 可命名判别）。

## 测试

- `dag_e2e::agent_runc::agent_step_session_runs_inside_runc_container`
  （真实 runc 1.1.12 全链路：容器内 session→output.json verdict=ok→
  transcript/session.json/meta.json 一致→knowledge 只读且无写痕→
  bundle process.args 与 ro 挂载断言；无 runc 时 SKIP）。
- 既有回归：`cargo check -p opencoder-dag-runtime --all-targets`、
  `cargo test -p opencoder-dag-runtime --lib`（57 通过，workspace 并发在途的
  wasm host_imports 2 个新测试除外）、`cargo check --test dag_e2e`。

Commit: (working-tree, pre-initial-commit)
