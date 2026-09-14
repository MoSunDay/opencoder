Commit: 56cc4b28e46dcc38ad2d3ab798413940c3155f93

# DAG wasm 显式版本 pin：`tool@v3.wasm` token 语法 + 受理冻结采纳

上迭代（`1ccb120e` 模块池）遗留后续项之一：节点 spec 此前只能写
`tool.wasm`，受理时冻结池 current，无法指名版本。本迭代补齐显式 pin：
spec 模块 token 新增 `name@v<n>.wasm` 形态，受理时冻结该不可变版本目录
的 `wasm.bin`，此后池发布/回滚任意翻转 current 都不影响该 run。同时交付
节点侧 `dag.wasm_dir` NFS 挂载运维文档（评审遗留项 2）与 control index
header 哈希回填（评审遗留项 4）。

## 变更

- **token 语法（新 `crates/dag-wasm/src/token.rs`）**：纯函数
  `parse_module_token` — `tool.wasm` → `("tool", None)`（pin current）；
  `tool@v3.wasm` → `("tool", Some(3))`。`v<n>` 规则：n≥1、无前导零、纯
  ASCII 数字、u32 范围；名称仍过 `validate_name`（嵌套/穿越路径与非池
  形态 token 一律 None → out-of-band，语义不变）。`crates/dag/src/spec.rs`
  的 `StepKind::Wasm` 文档补充该形态（DTO 未动，纯文档说明）。
- **pin 端采纳（`crates/worker/src/dag_wasm_pin.rs`）**：`pool_name` 删除，
  循环改用 `parse_module_token`；显式版本以 `meta.history.contains(&v)`
  判存在——池缺该版本与缺名同语义（跳过，保留 out-of-band），版本存在但
  导出损坏（sha256 不符）仍 fail-closed 拒绝受理。冻结落盘文件名 = 完整
  token（`_modules/tool@v1.wasm`），dag-runtime `resolve_module` 按 token
  文件名解析，**执行侧零改动**；`tool.wasm`（current）与 `tool@v1.wasm`
  （显式）可在同一 spec 并存，各自冻结。
- **运维文档（`docs/agent-platform.md`）**：NFS 章节补第二路导出（端口
  2050、`dag.nfs.enabled`、导出根 `dag.wasm_dir`/`<data>/dag/wasm`）、
  节点挂载命令样例与 `opencode.json` 配置；明确该路径不强制挂载表校验、
  三态语义（未配置 no-op / 缺名缺版本跳过 / 损坏拒绝受理）。DAG wasm 段
  补模块池分发与显式 pin 一句索引。
- **文档回填**：`agents/control/index.md` header 由并行会话遗留的
  working-tree 标记回填为 `bf757d2e`。

## 测试清单（规则 01/03：unit + e2e 分层）

| 主题 | 用例 | 位置 |
|---|---|---|
| token 语法（unpinned/pinned/非池/畸形 pin） | `unpinned_tokens_map_to_a_pool_name`、`pinned_tokens_map_to_name_and_version`、`non_pool_tokens_are_rejected`、`malformed_version_pins_are_rejected` | `crates/dag-wasm/src/token.rs` |
| 显式 pin 冻结指定版本而非 current（含 rollback+再发布后重受理不变） | `pin_freezes_the_explicitly_pinned_version_not_current` | `crates/worker/src/dag_wasm_pin.rs` |
| current 与显式版本并存各自冻结 | `pin_stages_current_and_explicit_versions_side_by_side` | `crates/worker/src/dag_wasm_pin.rs` |
| 池缺该显式版本 → out-of-band 跳过 | `pin_skips_explicit_versions_unknown_to_the_pool` | `crates/worker/src/dag_wasm_pin.rs` |
| e2e：显式 pin → wasmtime 实际执行 v1 → current 连续翻转（v3 发布+回滚）仍执行 v1 | `explicitly_pinned_version_executes_and_ignores_current_flips` | `crates/worker/tests/dag_wasm_pin.rs` |

## 回归（规则 02）

- 验证方法：并行会话 WIP 正在主 worktree 编辑 core/fleet（瞬时不可编译），
  故在隔离 worktree（`8b260caf` + 仅本次变更）上执行全量门禁，评审对象与
  验证对象严格同一。
- `cargo test --workspace --locked --no-fail-fast` → 381 套件
  **5137 passed / 0 failed / 5 ignored**（含 `web_project_runs` 时序套件，
  本次满载下未抖动）。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` →
  **0 警告通过**。

## 后续项

- 向仓库所有者反馈（评审遗留项 3）：`opencoder-web::web_project_runs`
  10s 轮询 deadline 的时序用例在满载机器易抖动（`1ccb120e` 门禁曾 3 例
  假阳、独立重跑全绿），建议硬化 deadline 或拆分慢机通道 — owner:
  仓库所有者。
- `agents/dag-wasm/index.md` 与本 changelog 的 header 哈希在紧随的 docs 提交中回填。
