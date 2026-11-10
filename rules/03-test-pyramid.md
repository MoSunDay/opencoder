# 规则 03：测试分层与组织

## 分层模型

```
      /e2e\          真 LLM（scripts/e2e-glm.sh），手动 / CI 触发
     /------\        判层看是否依赖真外部服务，非路径名
    /integration\    crates/*/tests/*.rs + 根包 tests/*_e2e/
   /--------------\  MockChatClient + tempdir + TursoStore::open_memory()
  /     unit        \ crates/*/src/*.rs 内联 #[cfg(test)]，纯函数毫秒级
```

### 第 1 层：单元测试（unit）

- **位置**：源文件内联 `#[cfg(test)] mod tests`
- **对象**：纯函数、无副作用逻辑、数据转换（如 `tokens::estimate`、`config::Config::load`）
- **要求**：零 I/O、零网络、零全局状态；`< 10ms` / 个

### 第 2 层：集成测试（integration）

- **位置**：`crates/<crate>/tests/*.rs`（含目录目标 `tests/<dir>/main.rs`）
- **对象**：跨模块协作、持久化、Mock 驱动的执行流程
- **要求**：用 `MockChatClient`（非真网络）、tempdir（非真文件系统）、`TursoStore::open_memory()`（非真数据库文件）
- 根包进程级形态（`tests/operator_e2e/`、`tests/dag_e2e/` 等）：真二进制子进程 + loopback LLM 桩，零外部服务 → 仍判第 2 层

### 第 3 层：端到端测试（e2e）

- **判层标准**：是否依赖真外部服务（真 LLM API key、真网络依赖）；**不以路径名判层**——随常规 `cargo test` 运行、Mock 驱动的 `tests/e2e/` 目录目标（如 control）属第 2 层
- **位置**：`scripts/e2e-glm.sh`（典型形态）
- **对象**：真 LLM、真文件系统、完整用户流程
- **要求**：需要 API key；手动 / CI 专属；不在常规 `cargo test` 中运行

## 放置决策树

```
新功能有副作用吗？
├── 否 → 第 1 层（源文件内联）
└── 是 → 跨多个模块吗？
    ├── 否 → 第 2 层（tests/ 目录，用 Mock）
    └── 是 → 需要真外部服务吗？
        ├── 否 → 第 2 层（tests/ 目录，用 Mock + tempdir）
        └── 是 → 第 3 层（判层看服务真伪、非路径名；典型如 scripts/e2e-glm.sh）
```

## 禁止行为

- 单元测试打开文件 / 网络 / 数据库
- 集成测试调用真 LLM API（用 `MockChatClient`）
- 把所有测试堆进巨型 `tests/main.rs`（按功能拆分文件）
- 测试文件超过 400 行（拆分或提取 helper）

## 具名测试取证规范

- 独立测试**文件**（`tests/<file>.rs` 或目录目标 `tests/<dir>/`）必须按目标运行：`cargo test -p <crate> --test <file>`；按函数名过滤（`cargo test <name>`）在无同名函数时对所有目标静默零命中（`0 passed; N filtered out`），不得当作通过证据。
- 目录目标用例名带模块前缀（`模块名::函数名`），按尾部片段过滤即可匹配。
- 取证前确认对象是「测试文件」还是「测试函数」；输出无 `test result:` 行即为无效取证。

## Mock 使用规范

- `MockChatClient`（`crates/llm/src/mock.rs`）：FIFO 脚本回放 + 请求体录制
- `TursoStore::open_memory()`：内存 SQLite，零文件残留
- `tempfile::TempDir`：文件系统操作的隔离临时目录
- 只 Mock 外部边界（LLM、文件系统、网络），不 Mock 内部类型

## 测试组织与目录落位

判层（上文）之后按本节落位。低频扰动原则：存量测试不强制迁移，触碰时归队；地图只锁目录级事实，不要求旧平铺文件搬进新形态。

### 模块 → 测试位置地图

| crate | integration 形态要点 |
|-------|----------------------|
| `core` | 平铺 + 目录目标 `skill_contract/`、`config_overlay/` |
| `llm` | 平铺 + `responses/`；全仓共享 `MockChatClient` 在 `crates/llm/src/mock.rs` |
| `store` | 平铺 + 多目录目标；多为「单文件目标 + 同名目录 `#[path]` 拆分」形态 |
| `event` / `database` / `todos` / `agents` / `local` / `dag` / `dag-binary` | 平铺；dag 为纯域模型，DTO LOCKED 线协议由 dag-runtime / dag-binary 覆盖 |
| `session` | 平铺 + 多目录（`responses/ harness/ common/ windows_native/ no_session_row_side_effects/`）；新夹具标准位 `tests/support/mod.rs` |
| `shellguard` / `server` | 仅内联 unit，无 `tests/` |
| `tui` / `ontology` / `team` | 平铺 + 存量夹具目录 `common/` 保留 |
| `web` | 平铺 + `support/ node_e2e_support/ agent_resources/ web_drain_contract/ dag_api/ web_project/` |
| `dag-runtime` | 平铺 + 目录目标 `run_loop/ dynamic/`；共享 `support/` 经 `#[path]` 跨目标复用 |
| `brain` | 平铺 + `milestone/`（`#[path]` 拆分形态） |
| `project` | 平铺 + 目录目标 `replay/ plan_and_execute/` |
| `control` | 平铺 + 功能域目录目标 `e2e/`（main.rs 按模块挂载、一模块一 API 族，目录目标样板）及 `node_tls/ project_files/ node_keepalive/ ontology_catalog/ catalog_guard/` |
| `node` | 平铺 + `support/`（线协议 wire-stub，含 `support/dag.rs`） |
| `ctl` | 平铺 + `ontology_server/ project/`；唯一跨 crate 复用：`#[path]` 引 control e2e support（存量特例） |
| `worker` | 目录目标为主（Cargo.toml `[[test]]` 显式声明 `platform/ drain_health/ durable_execution/ durable_lifecycle/ internal_session_index/ brain_io/` 等）+ 平铺 + `support/ harness/` |
| 根包 `tests/` | 目录目标 `operator_e2e/ dag_e2e/ brain_e2e/ brain_layered_e2e/ team_e2e/ todos_e2e/ server_modes_e2e/` + 单文件 smoke / CLI 用例；真二进制子进程 + loopback 桩判第 2 层，共享 `tests/support/` |
| 脚本层 | 验收：`scripts/acceptance/<domain>/`（页面覆盖登记 `scripts/acceptance/ui/scope.js`，存量根目录散脚本保留原位）；e2e 第 3 层：`scripts/e2e-glm.sh` 等 |

模块详情见 [repo-memory.md](../repo-memory.md) 模块索引，代码是最终事实；本表只锁目录级防漂移。

### 目录结构约定

1. **首选功能域目录目标**：`tests/<domain>/main.rs` 挂载 `mod xxx;`（control / worker 形态），用例名带模块前缀，按目标运行 `cargo test -p <crate> --test <domain>`；目录目标膨胀时按 API 族 / 模块拆子文件，仍由 main.rs 挂载。
2. **平铺 crate 的单文件目标预计超 400 行**：用「同名目录 + `#[path]`」仓库惯例——`tests/<file>.rs` 保留为目标入口，拆分文件放同名目录（store / brain / control e2e 均为此形态，新拆分沿用）：

   ```rust
   // tests/store_integration.rs
   #[path = "store_integration/wal.rs"] mod wal;
   #[path = "store_integration/recovery.rs"] mod recovery;
   ```

3. **夹具命名标准**：新代码统一放 `tests/support/mod.rs`；存量 `common/`、`harness/` 保留不迁，不新增第三种命名；同 crate 内多目标共享夹具经 `#[path]` 引用（dag-runtime `support/` 形态）。
4. **跨 crate 共享现状**：全仓唯一跨 crate 测试库是 `MockChatClient`，其余夹具各 crate 自持；ctl→control 一处相对 `#[path]` 属脆弱存量特例，勿扩大使用。

### 新增 case 决策树

```
判层（上文）→ integration →
  1. 定模块：查「模块 → 测试位置地图」，锁定 crate 与既有 tests/ 目录
  2. 选结构：有功能域目录目标 → 新文件挂进 main.rs；无 → 平铺新文件；
     预计 >400 行 → 同名目录 + #[path] 拆分
  3. 复用夹具：先查 tests/support/（或存量 common/、harness/）与 MockChatClient，
     重复手写 store / router 夹具前必须先看能否复用
  4. 登记：changelog 附「功能 → 测试名 → 文件」（规则 02）；页面类同步 ui/scope.js
```

### 验收脚本归组

- 新验收脚本一律进 `scripts/acceptance/<domain>/`，配套 README / harness 与既有子目录保持一致，不散落在 `scripts/` 根目录。
- 页面类 case 必须同步 `scripts/acceptance/ui/scope.js`（`CASES` 数组 + `verifyCoverage`，[规则 05](05-ui-acceptance.md)）。
- 存量根目录散脚本不强制迁移，触碰时顺势归组。

### 组织类禁止行为

- 新增第三种夹具目录命名（`support/` 之外再造 `common2/` 之类）
- 在新文件里手写与 `tests/support/` 重复的 store / router / LLM 桩
- 为"整齐"批量迁移存量平铺测试（churn 原则，[规则 02](02-regression-gate.md)：只在触碰时迁移）
- 跨 crate 用相对 `#[path]` 新增共享（现仅 ctl→control 一处存量特例）
- 把测试登记进 Ontology（[规则 09](09-ontology-capabilities.md) 界定的是能力目录，测试不是能力）
