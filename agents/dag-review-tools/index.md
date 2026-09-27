Commit: 6b5b4d28e047faa5b8d8468d6733c68cfd1e8685

# dag-review-tools 模块

code-review 发布门禁 wasm 模块（`env_eob_up` / `env_baremetal_up` / `harness_runner`）的纯编排逻辑库：解析节点写下的 op 证据、装配 `<step_dir>/output.json`；重活（部署、harness 运行）一律经 `dag.ops` host import 委派给节点白名单命令，wasm 模块自身只做证据解析与结果装配。

## 索引
- `src/lib.rs` — op 证据契约与 output.json 装配：`parse_op_log`、`stages_pass`/`failed_stages`、`write_output_json`、`endpoint_of`
- `src/host.rs` — 节点白名单 `dag.ops` 命令的 host import
- `src/bin/` — 三个门禁 wasm 模块入口：`env_eob_up.rs`、`env_baremetal_up.rs`、`harness_runner.rs`

## 证据契约
`<step_dir>/ops/<op_id>.log` 首行为 `# <op_id> exit=N|killed(...)` 头，`-- output --` 行后为原始捕获输出（`lib.rs` `parse_op_log`）。

## 消费方
- [tests/dag_e2e/review_dags/](../../tests/dag_e2e/review_dags/mod.rs) — R1–R5 门禁链（R4 六段 agent 链上下文传递 + 五字段索引）
- [docs/ops/opencode-review-ops/](../../docs/ops/opencode-review-ops/README.md) — 运维脚本 `env_eob_deploy.sh`
