Commit: (working-tree, pre-initial-commit)

# 规则 11：测试组织与模块地图

## 定位

[规则 03](03-test-pyramid.md) 回答"放哪一层"（unit / integration / e2e），本规则回答"放哪个目录、用什么结构、复用哪些夹具"。判层之后，按本文的模块地图和结构约定落位。

低频扰动原则（low-churn）：存量测试不强制迁移，只在触碰时归队。地图锁的是目录级事实，不要求把旧平铺文件搬进新形态。

## 功能模块 → 测试位置地图（目录级）

| 模块 | unit 位置 | integration 位置 | 备注 |
|------|-----------|------------------|------|
| **foundation** | | | |
| `crates/core` | 内联 | `tests/`（16 平铺文件）+ 目录目标 `skill_contract/`、`config_overlay/` | 共享类型与 Config 契约 |
| `crates/llm` | 内联 | `tests/`（8 文件）+ `responses/` | 全仓共享 `MockChatClient` 在 `crates/llm/src/mock.rs` |
| `crates/store` | 内联 | `tests/`（63 平铺文件）+ 目录目标 `store_integration/ brain_store/ project_store/ project_tags/ project_summary/ schema_bootstrap/ store_migrations/ brain_layered_v4/ contracts/ inputs_recorded/` | 多为「单文件目标 + 同名目录 `#[path]` 拆分」形态 |
| `crates/event` | 内联 | `tests/`（4 文件） | 纯内存总线与触发执行 |
| `crates/database` | 内联 | `tests/`（5 文件） | 内嵌引擎与 busy 重试 |
| **runtime** | | | |
| `crates/session` | 内联 | `tests/`（≈109 平铺文件）+ 目录 `responses/ harness/ common/ windows_native/ no_session_row_side_effects/` | 新夹具标准位 `tests/support/mod.rs` |
| `crates/shellguard` | 仅内联 unit | —（无 `tests/`） | 纯分类器，无集成面 |
| **frontend** | | | |
| `crates/local` | 内联 | `tests/`（9 文件） | CLI 解析与分发 |
| `crates/tui` | 内联 | `tests/`（31 文件）+ `common/` | 存量夹具目录 `common/` 保留 |
| `crates/web` | 内联 | `tests/`（63 文件）+ 目录 `support/ node_e2e_support/ agent_resources/ web_drain_contract/ dag_api/ web_project/` | SPA 契约与 API 面 |
| **ontology** | | | |
| `crates/ontology` | 内联 | `tests/`（7 文件）+ `common/` | 目录挂载与图谱 |
| **workflow** | | | |
| `crates/dag` | 内联 | `tests/`（1 文件） | 纯域模型；DTO LOCKED 线协议由 dag-runtime / dag-binary 覆盖 |
| `crates/dag-binary` | 内联 | `tests/`（1 文件） | 线协议序列化 |
| `crates/dag-runtime` | 内联 | `tests/`（3 文件）+ 目录目标 `run_loop/ dynamic/` | 共享 `support/` 经 `#[path]` 跨目标复用 |
| **orchestration** | | | |
| `crates/brain` | 内联 | `tests/`（5 文件）+ `milestone/` | `#[path]` 拆分形态 |
| `crates/team` | 内联 | `tests/`（5 文件）+ `common/` | 存量夹具目录 `common/` 保留 |
| **workbench** | | | |
| `crates/project` | 内联 | `tests/`（3 文件）+ 目录目标 `replay/ plan_and_execute/` | |
| `crates/todos` | 内联 | `tests/`（13 文件） | |
| **platform** | | | |
| `crates/control` | 内联 | `tests/`（13 文件）+ 目录目标 `e2e/`（main.rs 按模块挂载，一个模块一个 API 族，36+ 模块）及 `node_tls/ project_files/ node_keepalive/ ontology_catalog/ catalog_guard/` | `e2e/` 是功能域目录目标的样板 |
| `crates/server` | 仅内联 unit | —（无 `tests/`） | 只读资源服务 |
| `crates/node` | 内联 | `tests/`（5 文件）+ `support/` | 线协议 wire-stub，含 `support/dag.rs` |
| `crates/ctl` | 内联 | `tests/`（17 文件）+ 目录 `ontology_server/ project/` | 唯一跨 crate 复用：`tests/server_node.rs` 经 `#[path = "../../control/tests/e2e/support/node.rs"]` |
| `crates/agents` | 内联 | `tests/`（4 文件） | NFS 共享池与引用卡 |
| **execution** | | | |
| `crates/worker` | 内联 | 目录目标为主（Cargo.toml `[[test]]` 声明 `platform/ drain_health/ durable_execution/ durable_lifecycle/ internal_session_index/ brain_io/` 等）+ `tests/`（41 文件）+ `support/ harness/` | `[[test]]` 显式声明路径 |
| **根级进程套件**（根包 `tests/`） | | | |
| 根包 `tests/` | — | 目录目标 `operator_e2e/ dag_e2e/ brain_e2e/ brain_layered_e2e/ team_e2e/ todos_e2e/ server_modes_e2e/` + 单文件 `daemon_smoke.rs nodes_smoke_proc.rs responses_cli.rs skill_seed_startup_wiring.rs running_mode_switch_e2e.rs tui_exit_restore_e2e.rs windows_tui.rs` | 真二进制子进程 + loopback 桩，判第 2 层（规则 03）；共享 `tests/support/`（fleet_proc / http_util / llm_stub / native） |
| **脚本层** | | | |
| 验收脚本 | — | `scripts/acceptance/<domain>/`（19 个功能子目录（另有共享 harness/），如 `project/ ui/ dag/ brain/ ontology/ todo_workbench/` …） | 页面覆盖登记在 `scripts/acceptance/ui/scope.js`；存量根目录散脚本保留原位，新脚本一律进子目录 |
| e2e（第 3 层） | — | `scripts/e2e-glm.sh` 等 | 判层与执行方式见 [规则 03](03-test-pyramid.md) |

模块详情见 [repo-memory.md](../repo-memory.md) 模块索引，代码是最终事实；本表只锁目录级防漂移。

## 目录结构约定（新增 case 选结构）

1. **首选「功能域目录目标」**：`tests/<domain>/main.rs` 挂载 `mod xxx;`（control / worker 形态），用例名带模块前缀（`模块名::函数名`），按目标运行 `cargo test -p <crate> --test <domain>`。目录目标预计膨胀时按 API 族或模块再拆子文件，仍由 main.rs 挂载。
2. **平铺 crate 的单文件目标预计超 400 行时**：用「同名目录 + `#[path]`」仓库惯例——`tests/<file>.rs` 保留为目标入口，拆分文件放同名目录：

   ```rust
   // tests/store_integration.rs
   #[path = "store_integration/wal.rs"] mod wal;
   #[path = "store_integration/recovery.rs"] mod recovery;
   ```

   store / brain / control e2e 均为此形态，新拆分沿用。
3. **夹具命名标准**：新代码统一放 `tests/support/mod.rs`；存量 `common/`、`harness/` 保留不迁，也不新增第三种命名。同 crate 内多目标共享夹具经 `#[path]` 引用（dag-runtime `support/` 形态）。
4. **共享测试工具现状**：全仓唯一跨 crate 测试库是 `MockChatClient`（`crates/llm/src/mock.rs`）；其余夹具各 crate 自持。跨 crate 复用目前仅 ctl→control 一处相对 `#[path]`，属脆弱存量特例，勿扩大使用。未来选项（现在不做）：workspace 级测试支撑 crate——待重复度跨过阈值再评估。

## 新增 case 决策树

```
规则 03 判层（unit / integration / e2e）
└── integration →
    1. 定模块：查本规则「模块 → 测试位置地图」，锁定 crate 与既有 tests/ 目录
    2. 选结构：有功能域目录目标 → 新文件挂进 main.rs；
       无 → 平铺新文件；预计 >400 行 → 同名目录 + #[path] 拆分
    3. 复用夹具：先查 tests/support/（或存量 common/、harness/）与 MockChatClient；
       重复手写 store/router 夹具前必须先看能否复用
    4. 登记：changelog 附「功能 → 测试名 → 文件」（规则 02）；页面类同步 ui/scope.js
```

## 验收脚本归组约定

- 新验收脚本一律进 `scripts/acceptance/<domain>/`，配套 README / harness 与既有子目录保持一致，不把新脚本散落在 `scripts/` 根目录。
- 页面类 case 必须同步 `scripts/acceptance/ui/scope.js`（`CASES` 数组 + `verifyCoverage`，[规则 05](05-ui-acceptance.md)）。
- 存量根目录散脚本不强制迁移，触碰时顺势归组。

## 禁止行为

- ❌ 新增第三种夹具目录命名（support 之外再造 `common2/` 之类）
- ❌ 在新文件里手写与 `tests/support/` 重复的 store / router / LLM 桩
- ❌ 为"整齐"批量迁移存量平铺测试（churn 原则，[规则 02](02-regression-gate.md)：只在触碰时迁移）
- ❌ 跨 crate 用相对 `#[path]` 新增共享（现仅 ctl→control 一处存量特例）
- ❌ 把测试登记进 Ontology（[规则 09](09-ontology-capabilities.md) 界定的是能力目录，测试不是能力）
