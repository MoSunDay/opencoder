Commit: (working-tree, 基于 b465f440)

# dag-wasm 模块

DAG wasm 模块池：名称/描述/版本/更新时间四要素的版本化共享库，
server 侧发布 + NFS 只读导出 + 节点受理时冻结分发，替代此前
out-of-band 手工投放 `<data>/dag/_modules/`。

## 关键路径

- `crates/dag-wasm/src/write.rs` — save_wasm_version/rollback_wasm/delete_wasm；
  `.tmp-v{n}.<pid>` 整目录原子换入；版本号单调永不复用（回滚后再发布仍取新号）
- `crates/dag-wasm/src/meta.rs` — `WasmPoolMeta`/`WasmVersionMeta`（字段全 serde
  default 前向兼容）；`wasm_root()` 解析链：task-local scope → 进程 override →
  `OPENCODER_DAG_WASM_DIR` → None（由 web/control 中间件注入）
- `crates/dag-wasm/src/validate.rs` — 名称规则镜像 validate_resource_name；
  wasm 魔数 `\0asm` + LE version==1 + 32MiB 独立上限
- 布局：`<root>/<name>/meta.json` + `<root>/<name>/v{n}/wasm.bin` + `v{n}/meta.json`
  （sha256/size_bytes/updated_at）
- `crates/web/src/api_dag_wasm.rs` — /api/dag/wasm CRUD + rollback + wasm.bin 下载
- `crates/web/src/api_dag_wasm_nfs.rs` — 第二 NFS 导出生命周期 + scope 中间件
  （config `dag.wasm_dir` → 默认 `<data>/dag/wasm`）
- `crates/web/src/nfs_exports.rs` — 命名多导出注册表（agents 2049 / dag-wasm 2050）
- `crates/worker/src/dag_wasm_pin.rs` — 受理冻结：spec wasm 首 token → 池
  `v{current}/wasm.bin` → sha256 校验 → staging+rename 到 `_modules/<token>`；
  未配置 wasm_dir 静默跳过，池缺名跳过（保留 out-of-band），配置了但损坏
  fail-closed 拒绝受理
- core config `dag` 块：`wasm_dir` + `nfs{enabled,host,port=2050,read_only}`

## 边界

- 池读写域逻辑在本 crate；HTTP/导出在 web；节点冻结在 worker——三段共享
  `opencoder-dag-wasm` 类型
- 发布/回滚只切 current 指针；版本目录永不删除（同 agents 池契约）
- 节点 spec 不写版本号：受理时取 current 冻结（`tool@v3` 显式 pin 为后续项）

## 相关

- [agents/dag](../dag/index.md) — StepKind::Wasm 与 `_modules` 保留字
- [agents/dag-runtime](../dag-runtime/index.md) — resolve_module 消费端（零改动兼容）
- [agents/agents](../agents/index.md) — 版本池/NFS 导出的先例
