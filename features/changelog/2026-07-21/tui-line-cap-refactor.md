Commit: (working-tree, pre-initial-commit)

# TUI 行数合规重构 + 配置测试 HOME 隔离

## 背景

上一轮 `/config` 与 `/model` 拆分、Provider CRUD、自定义 headers 等特性落盘后，`crates/tui/src/app.rs`（1116 行）与 `crates/tui/src/render.rs`（814 行）双双超过仓库规定的「迭代中文件 ≤ 800 行」上限。同时 `crates/session/tests/capabilities_and_tools.rs` 里新增的 `Config::save` 往返测试依赖 `Config::save_target`，后者会沿 `config_candidates` 解析到真实 `~/.opencoder/config.json`（已含可编辑 key），存在并行测试覆写开发者真实配置、以及相互串扰导致非确定失败的风险。本轮为纯结构性重构：把超限文件按职责拆入内聚子模块，并给配置写测试加上 HOME/XDG 隔离，零行为变更。

## 变更

### app.rs 拆分（1116 → 790）

`run_app` 事件循环沿用既有 `app_helpers` 的「自由函数 + `&mut` 引用」抽取模式，新增两个模块：
- **`crates/tui/src/app_loop.rs`**（382 行，新建）：`compute_display()`（每轮显示态派生，纯函数，返回 `DisplayState` 结构体，沿用原 `&ChatView` 借用语义不克隆）；`handle_model_outcome()`（`ModelOutcome::Save` 配置热重载）；`dispatch_command()`（`/` 命令分发）；`fold_ui_events()`（worker 事件折叠）。引入 `LoopFlow` 枚举（`Proceed`/`Redraw`/`Quit`）把被抽取块里原先的 `continue`/`break` 翻译成返回值，调用点映射回 `continue`/`break`。
- **`crates/tui/src/app_task.rs`**（265 行，新建）：`switch_session()`（`TaskOutcome::Pick` 会话切换仪式，~135 行、24 个局部变量）；`handle_clear_all()`（清空其他会话）。均在 `app.rs` 用 `#[path = "..."] mod app_loop; / mod app_task;` 声明。
- **`crates/tui/src/app.rs`**：两个 `TaskOutcome` 分支与各抽取块替换为精简调用点；移除随之失效的 import（`rebind_session`、`gate_clear_all`、`handle_command_key` 等）。

### render.rs 拆分（814 → 685）

- **`crates/tui/src/render.rs`**：移除 `render_queue_panel`（队列面板渲染）。
- **`crates/tui/src/queue_panel.rs`**（179 → 314）：接收 `render_queue_panel`（改 `pub(crate)`），补齐 `Frame`/`Style`/`Paragraph` 等 import。该函数只依赖 queue-panel 类型 + ratatui widgets + `composer` 辅助，归入此模块高内聚；函数体逐字不变，仅可见性 `fn → pub(crate) fn`。
- **`crates/tui/src/render_tests.rs`**：因 `render_queue_panel` 不再在 `render` 模块内，加 `use crate::queue_panel::render_queue_panel;`（测试通过 `use super::*;` 引入）。

### 配置写测试 HOME 隔离

- **`crates/session/tests/capabilities_and_tools.rs`**：新增 `HOME_MUTEX` 静态互斥锁 + `lock_home()` / `HomeGuard` RAII 守卫，把 `HOME` 与 `XDG_CONFIG_HOME` 同时指向测试 tempdir，保证 `Config::save_target()` 解析到的全局候选路径全部落在 tempdir 内，永不触碰真实 `~/.opencoder/config.json`；`Drop` 时恢复原值并最后释放锁。`config_save_load_round_trips_capabilities` 测试使用该守卫。

### 文档

- **`features/changelog/2026-07-21/config-model-split-provider-crud-headers.md`**：全量回归计数由「768」订正为实测「769」（此前数值为单次幸运跑的快照，与当前可复现结果不符）。

## 测试覆盖

本轮为纯结构性重构，无新增业务功能；抽取行为完全被既有 324 个 tui 库测试 + `app_tests.rs` 覆盖。

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 队列面板渲染（移动后回归） | 既有 `render_tests` 全套 | `crates/tui/src/render_tests.rs` |
| run_app 事件循环（抽取后回归） | `app_tests.rs` 既有用例 | `crates/tui/src/app_tests.rs` |
| Config::save 往返（HOME 隔离） | `config_save_load_round_trips_capabilities` | `crates/session/tests/capabilities_and_tools.rs` |

- 全量回归：`cargo test --workspace` → 769 passed / 0 failed / 0 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- 行数：app.rs 790（≤800）、render.rs 685（≤800）、app_loop.rs 382（≤400）、app_task.rs 265（≤400）、queue_panel.rs 314（≤400）

## Gate

| 项 | 变更前 | 变更后 |
|----|--------|--------|
| app.rs | 1116 行（超限） | 790 行 |
| render.rs | 814 行（超限） | 685 行 |
| clippy | 0 警告 | 0 警告 |
| 测试 | 769 passed | 769 passed |

## Impact Surface

- 对用户：无可见变化（纯内部重构，TUI 交互、渲染、键位、会话切换逻辑均不变）。
- 不影响：store / session 运行时 / web / CLI / LLM client 边界；无新增 pub API、无类型变更、无配置格式变更。
- 风险：低——抽取为机械性移动（已逐字校验函数体不变 + `LoopFlow` 控制流翻译正确：`Quit=>break` / `Redraw=>continue` / `Proceed=>{}`），769/769 测试无回归。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [既有相关 changelog：/config 与 /model 拆分](./config-model-split-provider-crud-headers.md)
