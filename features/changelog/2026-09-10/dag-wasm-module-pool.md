Commit: (working-tree, 基于 b465f440)

# DAG wasm 模块池：发布 API + NFS 只读导出 + 节点受理冻结分发

补齐 wasm 步骤（`StepKind::Wasm`，wasmtime 双沙箱）的分发缺口：此前模块进入
节点共享库 `<data>/dag/_modules/` 完全 out-of-band（只有测试代码手工投放），
全仓库没有上传/分发通道。本迭代以 agents 资源池为模板贯通整条链路：
**名称/描述/版本/更新时间** 四要素版本化发布 → 第二路 NFS 只读导出 →
节点受理时冻结到 `_modules/`（publish/rollback 不影响已接受 run）。

## 方案

### 池（新 crate `opencoder-dag-wasm`，crates/dag-wasm）

- 布局对齐 agents 池惯例：`<root>/<name>/meta.json` +
  `<name>/v{n}/wasm.bin`（固定名）+ `v{n}/meta.json`
  （version/description/sha256/size_bytes/updated_at）。
- 版本 `v{n}` u32 单调递增永不复用（`next = max(history ∪ {current}) + 1`，
  回滚后再发布仍取新号）；回滚只切 current 指针，版本目录永不删除。
- 写入算法复用 agents 池：`.tmp-v{n}.<pid>` 整目录 → 逐文件 atomic_write →
  `fs::rename` 原子换入 → `atomic_write_json(meta.json)`；失败清 temp 不留撕裂。
- 校验：名称规则镜像 `validate_resource_name`；内容校验 wasm 魔数 `\0asm` +
  LE version==1；独立 32MiB 大小上限（agents 池 1.5MiB 文本上限不适用二进制）。
- 根解析链：task-local scope → 进程 override → `OPENCODER_DAG_WASM_DIR` → None
  （web/control 中间件注入，测试用 override）。

### API（`crates/web/src/api_dag_wasm.rs`，control `#[path]` 复用自动挂载）

- `POST /api/dag/wasm`（创建，已存在 409）、`PUT /api/dag/wasm/:name`（发新版本）
- `GET /api/dag/wasm`（列表）、`GET /api/dag/wasm/:name`（详情+历史）、
  `GET /api/dag/wasm/:name/versions/:v/wasm.bin`（二进制下载，octet-stream）
- `POST /api/dag/wasm/:name/rollback`、`DELETE /api/dag/wasm/:name`
- 上传协议 JSON+base64（仓库惯例，axum 未启用 multipart）；错误映射沿 envs 惯例
  （NotFound⇒404 / AlreadyExists⇒409 / InvalidInput⇒400）。
- 鉴权：web/control 现有 bearer 全覆盖；control 面 role gate 给非 admin
  GET-only（列表/详情/下载可用，写全拒）。

### NFS 第二导出

- `api_agent_nfs.rs` 的进程级单例 `NFS_SLOT` 泛化为命名多导出注册表
  `crates/web/src/nfs_exports.rs`（agents 2049 / dag-wasm 2050），
  agents 端点行为不变。
- `GET|POST /api/dag/wasm/nfs` 生命周期；config 新增 `dag` 块：
  `dag.wasm_dir`（默认 `<data>/dag/wasm`，per-workdir data dir）+
  `dag.nfs { enabled, host, port=2050, read_only }`；serve 自动拉起。
- scope 中间件 `configured_dag_wasm`：`/api/dag/wasm*` 注入
  config 根（无配置回落 data-dir 默认），双 server 根隔离。

### 节点分发（`crates/worker/src/dag_wasm_pin.rs`）

- 受理预检（`dag_preflight::validate`，runc 检查之前、两种 sandbox 都走）：
  spec wasm 步骤首 token（如 `tool.wasm`）→ 池 `<name>/v{current}/wasm.bin` →
  sha256 校验 → staging+rename 冻结到 `<workflow_root>/_modules/<token>`。
- 语义对齐 `resources::pin`：未配置 `dag.wasm_dir` 静默跳过；池中无该名字跳过
  （保留 out-of-band 投放）；配置了但池损坏（缺文件/sha 不符）fail-closed 拒绝受理。
- 与 `resolve_module`（`exec/wasm/mod.rs`）零改动兼容：spec 的
  `command: "tool.wasm --flag"` 照常工作；同内容重复受理幂等跳过拷贝。

### 决策点（按推荐落地）

1. JSON+base64 上传（≤32MiB）而非 multipart；2. 第二导出实例，不动现有 agents
   挂载；3. 节点取 current 冻结（spec 版本 pin `tool@v3` 为后续项）。

## 取舍

- `_modules/<token>` 保存受理时的 current 版本；后续 accept 发布新版本会替换
  （in_process 执行从库路径直读，runc 执行时已拷贝进 run 树）——冻结保证针对
  server 侧 publish/rollback，不针对同节点后续受理。
- 下载 ≤32MiB 一次性读回而非分块流（artifact.rs 的分块面向节点 RPC 边界）。
- dag-wasm crate 依赖 `opencoder-agents` 的 atomic_write 原语而非复制算法。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| v1 落盘含 meta/sha256/size | `save_creates_v1_with_correct_files_and_meta` | `crates/dag-wasm/src/write.rs` |
| 版本单调、回滚后不复用 | `versions_are_monotonic_across_rollback` | `crates/dag-wasm/src/write.rs` |
| 名称/魔数/碰撞拒绝 | `save_rejects_bad_name_bad_bytes_and_collisions` | `crates/dag-wasm/src/write.rs` |
| 崩溃不留 .tmp 撕裂 | `failed_write_leaves_no_tmp_leftovers` | `crates/dag-wasm/src/write.rs` |
| 回滚未知池/版本/剪枝目录 | `rollback_rejects_unknown_pool_version_and_pruned_dirs` | `crates/dag-wasm/src/write.rs` |
| 删除幂等 | `delete_is_idempotent_and_rejects_bad_names` | `crates/dag-wasm/src/write.rs` |
| 名称矩阵 | `name_matrix` | `crates/dag-wasm/src/validate.rs` |
| 魔数/版本字节校验 | `valid_header_and_bad_magic` 等 3 例 | `crates/dag-wasm/src/validate.rs` |
| meta 前向兼容（未知字段） | `metas_default_and_tolerate_unknown_fields` | `crates/dag-wasm/src/meta.rs` |
| 根解析链 scope→override→env | `wasm_root_prefers_scope_then_override_then_env` | `crates/dag-wasm/src/meta.rs` |
| HTTP 创建/摘要落盘 | `create_writes_v1_meta_binary_and_digest` | `crates/web/tests/web_dag_wasm.rs` |
| 重复创建 409 | `duplicate_create_conflicts` | `crates/web/tests/web_dag_wasm_errors.rs` |
| 版本滚动+回滚不复用（HTTP） | `put_rolls_versions_and_rollback_never_reuses` | `crates/web/tests/web_dag_wasm.rs` |
| 列表/删除/400/404 面 | `list_surfaces_pools_with_current_version_meta` 等 4 例 | `crates/web/tests/web_dag_wasm*.rs` |
| 命名多导出启停复用 | `named_export_start_reuse_stop` | `crates/web/src/nfs_exports.rs` |
| control 根隔离（双 server） | `wasm_pool_publication_uses_configured_root_without_cross_server_leaks` | `crates/control/tests/dag_wasm_scope.rs` |
| 未配置回落 data-dir 默认根 | `pool_scope_defaults_to_workdir_data_dir_when_unconfigured` | `crates/control/tests/dag_wasm_scope.rs` |
| role gate 非 admin 只读 | `role_gate_wasm_pool_is_read_only_for_users` + role_gate 单测矩阵 | `crates/control/tests/dag_wasm_scope.rs`、`crates/control/src/role_gate.rs` |
| config dag 块默认/序列化 | `dag_defaults_empty_object_matches_default_impl` 等 2 例 | `crates/core/src/config/dag.rs` |
| config merge 纳管 dag 块 | `merge_dag_block_wasm_dir_and_nfs` | `crates/core/src/config/merge.rs` |
| token 收集/名字映射 | `module_tokens_takes_first_wasm_token_deduped`、`pool_name_maps_only_flat_wasm_tokens` | `crates/worker/src/dag_wasm_pin.rs` |
| 未配置/池缺名跳过 | `pin_is_a_no_op_without_a_configured_pool`、`pin_skips_modules_unknown_to_the_pool` | `crates/worker/src/dag_wasm_pin.rs` |
| 冻结与新受理替换 | `pin_freezes_the_pool_current_and_replaces_on_new_accept` | `crates/worker/src/dag_wasm_pin.rs` |
| 池损坏 fail-closed | `tampered_pool_binary_fails_closed_and_stages_nothing` | `crates/worker/src/dag_wasm_pin.rs` |
| e2e：池发布→冻结→wasmtime 实际执行→池再发布不影响冻结 | `pinned_module_executes_and_stays_frozen_across_pool_publishes` | `crates/worker/tests/dag_wasm_pin.rs` |

- 全量回归：`cargo test --workspace --locked --no-fail-fast` → **5117 passed / 3 failed**
  （run 全程机器 load ≈100–200 并发编译风暴）。3 例失败全部集中于
  `opencoder-web::web_project_runs`（10s 轮询 deadline 的时序用例，gate 内单套件
  耗时 176s）。门后单独复核：同一二进制独立重跑 **4/4 通过（3.4s 与 5.4s 各一次）**，
  判定为本机负载抖动而非回归；其余全部套件（含 nodes_smoke_proc 两进程冒烟、
  web dag-wasm 7 例、control 3 例、worker 冻结 e2e）全绿。
  - 修复 clippy 后重验：`cargo test -p opencoder-web`（全量，含改动后的
    `web_dag_wasm`/`web_dag_wasm_errors` 7/7）全绿。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **0 警告通过**
  （全 workspace，9m36s；含本迭代触达的 dag-wasm/core/web/control/worker 及全部测试目标）。
  门前提检曾报 7 处 `await_holding_lock`（测试 harness 的 `MutexGuard` 解构绑定跨
  await），已改为仓库既有惯例 `let _scoped = scoped();` 整元组绑定（对齐
  `web_agents.rs`）后归零；修复后重跑 `cargo test -p opencoder-web` 全量 55 个套件
  结果行全 ok（含改动后的 `web_dag_wasm`/`web_dag_wasm_errors` 7/7）。
