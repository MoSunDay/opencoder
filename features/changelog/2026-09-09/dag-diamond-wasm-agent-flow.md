Commit: (working-tree, 基于 b465f440)

# 菱形 workflow：wasm 步数据流 → agent 步汇总的 platform 端到端闭环

补齐 DAG 链路里「多步真实数据流」的缺口：此前 platform 用例只有单 agent 步闭环，步与步之间是否真的传递结构化输出无从断言。本轮在 `crates/worker/tests/platform/dag_diamond_flow.rs` 落一个菱形 workflow——`a`（wasm，写 `{"value":1}`）→ `b`（wasm，读 a 的 context.json 并 +1 → 2）/ `c`（wasm，读 a 的 context.json 并 +3 → 4）→ `d`（agent，读 a/b/c 传递依赖上下文汇总）。control 保存定义 → `/api/dag/defs/diamond/dispatch` → worker 节点真实执行：wasm 步由节点内嵌 wasmtime 跑测试内现场编译的 WASI 模块（`wat::parse_str` staging 到 `<data>/dag/_modules/`），agent 步走真实会话 + `MockChatClient` 脚本（`TextDelta`+`Completed` 成对，```json 围栏驱动 `extract_output_json_from`），零真实 LLM、零外部网络。

WAT 模块为纯手写 WASI preview1 调用：`b`/`c` 模块 `path_open`（oflags=0、rights=FD_READ=1<<1）只读打开自身 `context.json` → `fd_read` → **从尾向前扫描最后一个 ASCII 数字**（pretty 上下文中上游 `value` 是最后一个数字）→ 在 `{"value":0}` 模板占位字节上 splice `digit+delta` → `path_open`（CREAT|TRUNC、rights=FD_WRITE）写出 `output.json` 并 stdout 回显。读侧 rights 用 FD_READ（wasmtime 将该 bit 映射为 READ 打开标志）。

三层断言：① 四个步骤产物 `output.json` 分别为 `{"value":1}` / `{"value":2}` / `{"value":4}` / `{"total":6,"from":{"b":2,"c":4}}`（wasm 真算了、agent 围栏 JSON 真恢复了）；② `b/context.json` 内 `steps.a.json=={"value":1}` 且 `ok==true`（wasm 步真读了上游文件契约）；③ agent 步请求的 user prompt（含传递依赖 a/b/c 的 pretty 上下文）同时含 `"value": 1/2/4` 三个片段（agent 步真拿到了全链路输出）。

新增 `crates/worker/tests/platform/dag_diamond_flow.rs`（203 行）；`tests/platform/main.rs` 挂载模块。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 菱形 DAG：保存定义→dispatch→真实节点执行→done | `diamond_workflow_wasm_steps_feed_the_agent_step` | `crates/worker/tests/platform/dag_diamond_flow.rs` |
| wasm 步真实读写上游 context.json/output.json 文件契约 | `diamond_workflow_wasm_steps_feed_the_agent_step` | `crates/worker/tests/platform/dag_diamond_flow.rs` |
| wasm 步结构化输出进入下游（a→b/c 数值变换产物） | `diamond_workflow_wasm_steps_feed_the_agent_step` | `crates/worker/tests/platform/dag_diamond_flow.rs` |
| agent 步收传递依赖上下文（d 的 prompt 含 a/b/c 输出） | `diamond_workflow_wasm_steps_feed_the_agent_step` | `crates/worker/tests/platform/dag_diamond_flow.rs` |
| agent 步围栏 JSON 恢复为步骤 output.json | `diamond_workflow_wasm_steps_feed_the_agent_step` | `crates/worker/tests/platform/dag_diamond_flow.rs` |

## 验证结果

- `cargo test -p opencoder-worker --test platform`：全绿（新增 1 例；并行会话随后在同一 suite 追加了 operator 用例，最终 9 passed）。
- 定向回归 `-p opencoder-worker -p opencoder-control -p opencoder-dag-runtime`：全绿（含既有 manual ignore 项）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（本轮改动后复验 `-p opencoder-worker -p opencoder-store` 同样零警告；随后 `opencoder-session` 出现并行会话在途编译错误，与本轮无关）。
- 全量 `cargo test --workspace`：**361 个测试目标 4986 passed / 0 failed / 5 ignored**（5 项为既有手动环境用例）。
- 行数 gate：新增文件 203 行（≤400）；无硬编码凭据。

## 附注（并行会话协同）

- 全量 gate 期间并行会话正推进 schema v24（users 表）：其已改断言消息为 "24" 但漏改 8 个测试文件里 17 处字面量 `23`。本轮按其意图补齐 `crates/store/tests/`（display_text / inputs_recorded / project_store / schema_bootstrap / schema_v4_migration / store_migrations/{early,middle,sessions,project_replay}）的字面量为 24，`-p opencoder-store` 恢复 236 passed / 0 failed。
- 期间一次全量日志被共享 target 目录里的过期产物污染（出现早已退役的 `exec::python` 测试段），已识别弃用并以干净重跑为准。
