# todos drive 降级加固、verify 预算 min(主窗, judge 窗)、review iteration 0 起口径

基线 Commit: (working-tree, post-7a9f188)

## 背景

批次二 P2-5 的三处缺陷：todos `drive()` 错误路径自伤、autopilot VERIFY 快照按
主模型窗口预算但发给 small_model、review 事件迭代号 1 起与 drive 的 0 起口径不一致。

## 变更

- **todos `drive()` 探测降级 + 接管日志**（`crates/todos/src/runner.rs`）：错误路径的
  接管探测 `persistence::load` 原先 `?` 直通——探测自身失败会把**原始运行错误**顶掉
  且跳过整个 suspend 落盘流程（错误处理路径自身可崩溃，workflow 卡死 Running）。
  现降级为 `warn` + `None`：继续走本地 suspend 落盘，原始 error 保持传播。接管分支
  （远端 generation 不同且 Suspended → 采纳远端状态）补 `tracing::info!`
  （workflow_id、本地/远端 generation），便于排查双 runner 接管。
- **VERIFY 快照预算改 `min(主窗, judge 窗)`**
  （`crates/session/src/autopilot/verify.rs` + `crates/core/src/config/autopilot.rs`）：
  `build_snapshot` 原预算 `context_limit() - VERIFY_RESERVED_TOKENS` 用的是主模型
  窗口，但 verify 实际调用 `small_model`——小窗更小时快照溢出其窗口
  （HTTP 400 → Malformed 降级重试）。新增 `AutoPilotConfig.verify_context_limit:
  Option<u64>`（serde default None；merge 键 `autopilot.verify_context_limit`，后写
  覆盖；参照 `verify_retries` 写法），预算改
  `context_limit().min(verify_context_limit.unwrap_or(u64::MAX)).saturating_sub(VERIFY_RESERVED_TOKENS)`。
  未配置时行为不变。`config.rs` 主文件未动（仍 800/800），新代码全在
  `config/autopilot.rs`。
- **review iteration 口径统一 0 起**（`crates/session/src/autopilot/review_pass.rs`）：
  `REVIEW_ITERATION` 1 → 0，与 `drive` 的 `ApState::iteration`（0 起）及
  `should_stop` 的 `iteration + 1 >= max` 封顶算术一致。grep 全仓同步：tui 渲染
  （`autopilot: {:?} #{}`）与 web/cli 本就动态无硬编码；2026-08-16 changelog 中
  `AutoPilot { phase: Review, iteration: 1 }` 事件面文案更正为 0。上一批次
  （autopilot-verify-tighten-and-review-cancel）「明确不做」中的该项由本批次完成。

## 测试清单（每项先红后绿）

- **todos**（新 `tests/drive_degrade.rs`，含委托 `ProbeFailingStore` 包装——仅覆写
  `get_todo_workflow`：workflow 行存在时报 Err，模拟探测窗口内 store 故障）：
  - `probe_failure_degrades_and_original_error_survives`（红：返回错误为
    "store exploded during takeover probe" 而非原始 "mock exhausted"；绿：原始错误
    传播 + store 中 workflow 仍 Suspended + 末事件 `runtime_error`）。
    说明：现有 harness 无法注入 load 失败，故造委托包装；`persistence::load` 走
    `store.get_todo_workflow`。
- **core**（`src/config/autopilot.rs` 单元）：`verify_context_limit_defaults_none_and_merges`
  （Default/空对象 None、300 覆盖、后写覆盖 4000→300）。
- **session 单元**（`src/autopilot/verify.rs`）：
  - `narrow_verify_context_limit_truncates_snapshot_to_judge_window`（红：
    "judge window must actually truncate: snapshot 42 vs transcript 40"；绿：
    40×400 字符 transcript、主窗 100_000、judge 窗 4_000 → 截断且整快照估算 ≤ 4000）；
  - `unset_verify_context_limit_keeps_primary_budget`（回归锚：未配置时全量克隆
    不变）。
- **session 集成**（`tests/autopilot_review.rs`）：
  - `review_mode_runs_exactly_one_review_pass` 断言 `AutoPilot { Review, 0 }`
    （红：0 个匹配，事件仍 iteration 1；绿）；
  - `ap_mode_with_max_iterations_one_still_cycles_phases` 补 drive 0 起锚
    （单周期全部事件 iteration == 0，`[0, 0, 0]`）。

## Gate

- `cargo test -p opencoder-todos`：38+38 单元 + 6/1/8/7/2/7/0 集成全绿（含新增
  drive_degrade 1）。
- `cargo test -p opencoder-core`：169 单元 + 全部集成绿。
- `cargo test -p opencoder-session`：85 个测试二进制全绿（共享树，含并行批次改动）。
- `cargo clippy -p opencoder-todos -p opencoder-core -p opencoder-session
  --all-targets -- -D warnings`：0 警告。
