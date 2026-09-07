Commit: c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6（开发基线；多轮工作树成果随本提交落地）

# DAG wasm 收敛评审遗留项消化（迁移报错统一 / `_modules` 互作 / runc manual 实跑）

## 问题与行为

上轮「DAG 步骤收敛为 agent/wasm + how_append」评审确认可上线，但留下 6 项遗留 TODO。本轮全部消化：

1. **python 迁移哨兵全入口统一**：新增 `opencoder_dag::decode_spec_str`（字符串入口，parse 后走 `decode_spec`）。project `executor/dag_drive.rs`（todo 内联 spec + def.spec_json 两处）与 web `api_dag.rs`（`post_def` 改为 raw Value 反序列化 → 专用 400 文案；`def_view` 读路径）全部接入，存量 python 定义在任何路径都得到「该定义使用已下线的 python 步骤，请改写为 wasm/agent 后重新保存」，不再是 raw serde unknown variant 报错。
2. **web 定义列表降级而非毒化**：`list_defs` 对无法解码的 def 降级为 `{id,name,created_at,updated_at,error}` 行（好行仍精确展开为 `DagDefView` wire 形状，LOCKED 协议 DTO 未动），一条旧记录不再把整页 500；SPA defsTab 对坏行显示「定义无法解析」Tag 并禁用 派发/编辑（删除保留供清理）。
3. **`_modules` 与 worker 布局迁移互作**：`migration.rs` 的 legacy workflow 根校验把 `_modules` 与 `rootfs`/`bundles` 同列豁免——legacy 布局节点使用共享模块库后仍能完成布局迁移（此前直接 bail "legacy data has no execution record"）。
4. **`_modules` 保留 run id**：`artifacts::validate_run_id` 拒绝恰好为 `_modules` 的 run id（与模块库目录冲突会得到费解的 409/目录已存在错误），失败前移到统一校验点。
5. **SPA stepInspector 补 agent 的 `how_append` 输入**（校验器 8KB 镜像早已就绪），编排面与 spec 能力对齐；`exec/agent.rs` 陈旧的 "python step would see" 注释改为 wasm `context.json` 契约表述。
6. **runc manual 测试实跑闭合**：新增 `crates/dag-runtime/examples/wasmtime-cli`（复用执行器同款 wasmtime/wasi crate 的最小 CLI，支持 `run --dir[=host::guest] --env K=V <module> [args…]`、stdio 继承、guest 退出码透传）与 `scripts/prepare-dag-rootfs.sh`（scaffold + CLI + ldd 依赖镜像进 rootfs）。本机 runc 环境实跑 3 个 `#[ignore]` manual 测试全部通过——上轮唯一未运行验证的执行路径已闭合。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| `decode_spec_str` 哨兵/语法错误/正常解码 | `decode_spec_str_maps_python_and_syntax_errors` | `crates/dag/src/spec.rs` |
| `_modules` run id 拒绝 | `run_id_rejects_traversal` 扩展 | `crates/dag/src/artifacts.rs` |
| wasmtime-cli argv 解析（bundle 形状/`host::guest`/未知 flag fail-closed） | `parses_the_bundle_argv_shape` | `crates/dag-runtime/examples/wasmtime-cli.rs` |
| project 内联/引用 python spec 专用报错 + 无悬挂 session | `dag_executor_python_spec_fails_with_dedicated_migration_error` | `crates/project/tests/executor_team_dag_brain.rs` |
| web：python spec 400 专用文案、坏行降级（保 id/name/timestamps）、get_def fail-closed、好行无 error 字段 | `python_defs_fail_closed_with_dedicated_errors` | `crates/web/tests/dag_api.rs` |
| 布局迁移豁免 `_modules` 且保留库内容 | `migration_copies_and_verifies_typed_tree_while_retaining_legacy` 扩展 | `crates/worker/tests/layout_migration.rs` |
| SPA：agent how_append 字段渲染/提交、wasm 步不出现 | editor dom 测试 +2 | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| SPA：坏行 Tag + 派发/编辑禁用、删除可用 | defsTab dom 测试 +1 | `crates/web/spa/src/dag/dag.dom.test.jsx` |
| runc manual 3 项（smoke/取消超时回收/输出溢出回收） | `--ignored` 实跑 3/3 ok（runc 1.1.12 + 脚本投放 rootfs） | `crates/dag-runtime/src/sandbox/runc.rs` |

## 验证

- `DAG_TEST_ROOTFS=… cargo test -p opencoder-dag-runtime --lib sandbox::runc -- --ignored`：3 passed / 0 failed（真实 runc 1.1.12，`runc --version` 实测）。
- `cargo test --workspace --locked --no-fail-fast`：exit=0，314 个测试二进制 / 332 条 result（含 doc-test）全部 ok，合计 4798 passed / 0 failed / 5 ignored（其中 3 个 runc manual 已另行实跑；+1 二进制/+1 用例即 `wasmtime-cli` example 测试纳入默认回归）。
- example 单测：`[[example]] test = true` 后 `wasmtime-cli` 的 argv 解析测试随 `cargo test --workspace` 默认执行（此前仅 `--examples` 显式运行）。
- SPA 460 用例绿、`check-spa-drift.sh` 无漂移。

## 追加修复（同工作树评审自查）

- **`list_defs` 降级行丢 id（D1）**：原实现 `DefListRow` 用 `#[serde(flatten)] view: Option<DagDefView>`，serde 对 `None` 的 flatten 不输出任何键，坏行落线只剩 `{"error": …}`——SPA 删除按钮将请求 `/api/dag/defs/undefined` 而 404。web 测试只断言 `spec==null && error!=null`、SPA dom 测试喂的是后端从不产出的带 id 形状，双侧绿灯掩盖。改为显式 `json!({id,name,created_at,updated_at,error})`（好行仍精确序列化为 `DagDefView` wire 形状，LOCKED 协议 DTO 未动），web 测试补断言 id/name/created_at/updated_at。
- **example 单测纳入默认回归**：见上文验证一节。
