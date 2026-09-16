# 节点调度配置新增 workdir 工作空间

## 行为

- 节点列表「调度配置」弹窗新增可选字段「工作空间（workdir）」：为该节点的 opencoder 指定工作空间目录（必须为绝对路径）。
- 生效范围：设置后该节点上 opencoder 会话（agent/maintenance/operator/team/todo/dag/project 等非 brain 工作负载）以该目录为工作空间——会话工作目录、配置发现（`Config::load`）与会话归属（`workdir_hash`）都随工作空间；brain 工作负载继续使用各自执行目录的 `workspace`。节点自身数据目录（runtime.db、队列 journal）与启动 workdir 不受影响。
- 留空/清空即恢复节点启动目录；保存时控制面预校验绝对路径，节点侧 fail-closed：保存前创建目录（失败拒绝保存），启动时再次确保目录存在（缺失仅告警，不阻断节点启动）。
- 配置持久化在节点 `scheduling.json`，重启后继续生效。多 runtime Host 不支持 workdir（configure 返回 400）。
- 新增 `GET /api/nodes/:id/scheduling`（admin-only）返回当前 NodeScheduling（max_runs/queue_order/workdir）；`PUT` 在原语义上扩展 workdir（空白值归一为 null）。节点快照与协议版本不变。

## 实现要点

- `crates/core/src/fleet/queue.rs` — `NodeScheduling.workdir: Option<String>` + `normalized()` + `validate()`（绝对路径）；失去 `Copy`。
- `crates/worker/src/brain/workdir.rs` — 生效工作空间缝隙 `node_workdir()`；`for_record()` 非 brain 记录与 `native_state()` 内部 API 都以生效工作空间构建。
- `crates/worker/src/state.rs` — `configuration()` 读生效工作空间配置；启动时确保工作空间存在。
- `crates/worker/src/workloads/agent.rs`、`operations/create.rs` — 会话 workdir_hash、resume drain、codex 二进制预检解析都使用生效工作空间。
- `crates/worker/src/operations/maintenance.rs`、`crates/agent/src/host/service.rs` — configure_scheduling 归一化校验；host 模式拒绝 workdir。
- `crates/control/src/api/settings/mod.rs` + `routes.rs` — `GET/PUT /api/nodes/:id/scheduling`。
- SPA `crates/web/spa/src/fleet/settings/scheduling.jsx` — 工作空间输入（GET 回填、绝对路径校验、清空提交 null）；dist 已重建。

## 验证

（待回归后回填）

- 编译与静态检查：全 workspace `cargo check` 通过；`cargo clippy -p opencoder-core -p opencoder-worker -p opencoder-agent -p opencoder-control --all-targets` 零告警。
- `cargo test -p opencoder-core --lib` — 257 passed，含新增 `fleet::queue::tests::workdir_is_optional_absolute_and_blank_clears`（可选 workdir、空白清空、绝对路径校验）。
- `cargo test -p opencoder-control --lib` — 57 passed（GET/PUT scheduling 序列化与 API 行为）。
- `cargo test -p opencoder-agent` — 9 passed（多 runtime Host 拒绝 workdir）。
- `cargo test -p opencoder-worker --lib` — 56 passed（生效工作空间配置、启动建目录）。
- `cargo test -p opencoder-worker` 23 个集成测试文件全部通过（合计 61 passed、1 ignored 为既有 NFS 条件跳过）：新增 `scheduling_workdir` 2 例（相对路径 400、保存即建目录、GET 回读、会话 cwd=workspace、清空恢复、重启后 scheduling.json 生效）；`harness_settings_queue` 4 例（调度弹窗字段与提交）；`workloads` 5、`project_replay` 10、`dag_wasm_pin` 10、`resource_snapshot` 6、`layout_migration` 4、`brain_ontology` 3、`brain_recovery` 2、`todo_review` 2、`initial_input_recovery` 2 及其余单例文件。
- SPA：`management.dom.test.jsx` 6 例通过（GET 回填 workdir、绝对路径校验、清空提交 null）；`npm run build` 产物刷新；`scripts/acceptance/harness/settings.js` 浏览器验收通过（验收后清空恢复）。
- 环境说明：共享 cargo 环境（/data00/rust-build）被并行会话长期持有文件锁，本次改用隔离 CARGO_HOME/TARGET_DIR 分片执行；`cargo test --workspace` 单次扫描受机器负载（峰值 load>300）限制未能在单窗口完成，以全 workspace 编译检查 + 全部受影响 crate 完整测试套件替代，未涉及 NodeScheduling 的其余 crate 已 grep 确认零引用。
