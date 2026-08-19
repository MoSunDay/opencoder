Commit: (working-tree, post-7a9f188)

# TUI 批次三收尾：cmd 通道 try_send 防冻结 + disabled 白名单补全 + render/注释卫生

## 背景（4 个子项）

1. **cmd 通道灌满冻结 UI**：worker cmd 通道为有界 `mpsc::channel::<UiCmd>(4)`（`app_bootstrap.rs`）。`handle_switch_agent` 纯切换分支的 `cmd_tx.send(UiCmd::SwitchAgent(name)).await` 在通道满时**阻塞 UI 事件循环**——worker 忙于长 turn 不消费、UI 侧 running 尚未回传的窗口内连按切换键 → 灌满 → 整个 TUI 冻结在 `.await` 上。
2. **disabled 白名单漏 `bindings.switch_mode`**：P1-1 放行了 `input_disabled`（subagent-focus）视图的 `switch_mode_clear` / `switch_mode_keep` / raw BackTab，但漏了 `switch_mode`（默认 ctrl+t，`KeyAction::SwitchAgentNoClear`）——该自定义绑定在 subagent-focus 视图下失效，与「离开/切换模式永不被视图状态阻断」的 P1-1 裁决矛盾。
3. **陈旧注释 + render 芯片误染**：`app.rs` 的 `SwitchAgentNoClear` 臂注释仍写「mode switch mid-turn defers to the next idle boundary」——双向拦截后语义是「running 中拦截并给 busy 提示」；`render.rs` 的 `text.contains("plan")` 让任何未来含 "plan" 的中性 flash 都被误染成 plan 色（busy flash 是靠「刻意不含 plan 子串」才侥幸正确，契约脆弱）。
4. **changelog 行数勘误**：P1-1 文档的行数段停留在其时点数字。

## 变更

### 子项 1：`try_send` + 连续同名去重（防通道灌满冻结 UI）

- **`crates/tui/src/worker.rs`**（纯函数，无类无状态）：
  - `pub fn dedup_switch(prev: Option<&UiCmd>, next: &UiCmd) -> bool`：连续同名 `SwitchAgent(name)` → true（重复应丢弃）；异名 / 首次（`None`）/ 非 `SwitchAgent`（尤其 `SwitchAndStart`——它启动 turn）→ false 永不去重。
  - `pub fn try_send_idempotent(tx, cmd) -> bool`：尽送 helper——`try_send` 永不等待容量；`TrySendError::Full` warn 日志并丢弃（**只对幂等命令安全**：重发到达相同状态，UI chip 已乐观折叠、idle 重按即重发）；`Closed` 与原 `let _ =` 同语义返回 false。
  - `UiCmd` 补 `Clone`（dedup 基线在 UI 侧克隆保存）。
- **`crates/tui/src/app_loop.rs`（`handle_switch_agent`）**：纯切换分支改 `dedup_switch` + `try_send_idempotent`，成功入队才记录 `last_switch_sent` 基线（丢弃不记录——通道腾空后重按可真正送达）；`SwitchAndStart` 交接分支启动 turn 后同步清空基线（避免「纯切 plan → 交接 act → 再纯切 plan」被错误折叠）。新增 `last_switch_sent: &mut Option<UiCmd>` 参数，由 `app.rs` 循环本地持有。
- **不动**（有意保留 `.await`）：`start_turn`（ResetCancel+turn cmd，丢弃会造成 running=true 假死）、`UiCmd::Quit`（丢弃退不出去）、`Prompt`/`EditPlan`/`EditAnnotation`/`ReloadConfig`（丢弃即丢用户动作）。slash 路径 `dispatch_mode_switch` 不发纯 `SwitchAgent`（走 `Prompt`/`SwitchAndStart`），无需改。

### 子项 2：disabled 白名单补 `bindings.switch_mode`

- **`crates/tui/src/key_handler.rs`（`input_disabled` 分支）**：按同款模式补放行，返回 `SwitchAgentNoClear(next)`（与 enabled 路径 ctrl+t 臂同语义）；裁决仍在 `handle_switch_agent` 双向 running 门。

### 子项 3：陈旧注释 + render 芯片误染

- **`crates/tui/src/app.rs`（`SwitchAgentNoClear` 臂）**：注释同步双向拦截语义（busy → 拦截 + busy 提示，re-press when idle，never deferred）。
- **`crates/tui/src/render.rs`**：`is_plan` 改为 `text.starts_with("→ plan mode")`——**只有确定的模式切换 flash 参与双色**；busy 提示、`→ act mode`、任何含 "plan" 的中性文本一律 accent。契约写入注释。

### 子项 4：changelog 行数勘误

- `plan-switch-direction-aware-running-gate.md` 行数段按批次三完成后 `wc -l` 实测重算（app_loop.rs 800/800 等），「均 ≤800」表述与实际一致。

## 测试覆盖（先红后绿）

| 功能 | 测试名 | 文件 | 断言要点 |
|------|--------|------|----------|
| 通道灌满不阻塞（红：`Elapsed(())`） | `pure_switch_returns_promptly_when_cmd_channel_is_full` | `crates/tui/src/app_loop_tests/switch_gate_tests.rs` | 预塞满 capacity=4 通道 → idle 纯切换在 750ms timeout 内返回 Proceed、不 panic；flash 正常、chip 乐观折叠、running=false；通道仍恰含 4 条种子（丢弃未入队） |
| app_loop 层去重 | `pure_switch_dedup_drops_consecutive_same_name_repeat` | 同上 | 基线=同名 `SwitchAgent("plan")` → 重复被本地丢弃（无通道流量）但 flash 照常、chip 照折；基线=异名 → 正常发送 |
| 去重纯函数 | `dedup_switch_drops_consecutive_same_name` / `dedup_switch_allows_different_name` / `dedup_switch_allows_first_send` / `dedup_switch_never_drops_switch_and_start` | `crates/tui/src/worker/tests.rs` | 同名丢弃 / 异名放行 / None 首次放行 / `SwitchAndStart` 永不去重（双向） |
| 尽送 helper | `try_send_idempotent_enqueues_when_capacity_remains` / `try_send_idempotent_drops_without_awaiting_when_full` / `try_send_idempotent_reports_closed_channel` | 同上 | 有容量入队 true / 满通道丢弃 false 不等待 / 关闭通道 false |
| disabled 视图 switch_mode 绑定（红：`got None`） | `handle_key_disabled_allows_switch_mode_binding` | `crates/tui/src/key_handler_disabled_mode_tests.rs` | ctrl+t 在 disabled 视图返回 `SwitchAgentNoClear`（双向 act↔plan），input 不被触碰 |
| 旧契约测试改写 | `ctrl_t_blocked_when_input_disabled` | `crates/tui/src/app_tests/key_tests.rs` | 原断言 `None` 与 P1-1「模式切换键不被视图状态阻断」矛盾（其引据 "matching Ctrl+Shift+Tab's behaviour" 已过时）——改为断言 `SwitchAgentNoClear("act")` |
| render 双色契约（红：中性文本被误染 Yellow） | `mode_flash_chip_two_colour_only_for_definite_switch` | `crates/tui/src/render_tests/chips.rs` | 全帧渲染断言：`→ plan mode` → warn(plan) 色；`→ act mode` / busy flash / "plan submitted"（中性含 plan）→ accent 且非 warn 色 |

## 回归

- 红（先测后改）：子1 通道灌满 `Elapsed(())`（阻塞复现）；子2 `ctrl+t must switch mode from the disabled view, got None`；子3 `flash "plan submitted" must render on Cyan bg; got [... Yellow ...]`（contains("plan") 误染复现）。
- 绿：`cargo test -p opencoder-tui --no-fail-fast` → **1530 passed / 1 failed**（1461 lib + 69 integration；唯一失败 `queued_skill_fires_at_consumption_not_during_kickoff` 为并行批次在 75d6866 已提交的 skill 生命周期改动引入，隔离 worktree 于干净 HEAD 复现同样失败，与本批次无关）；`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 0 警告 / EXIT=0。
- 行数：`app_loop.rs` 800 / 800（+6：签名+1、纯切换分支+5，helper 全在 worker.rs）；`app.rs` 800 / 800（+1 声明，两调用点原地扩参）；`render.rs` 800 / 800（+2）；`worker.rs` 690 / 800；`switch_gate_tests.rs` 759 / 800（均 ≤800）。

## Impact Surface

- TUI 用户：worker 忙时连按模式切换键不再冻结 UI（多余的幂等切换被丢弃并 warn，idle 重按即生效）；subagent-focus 视图下自定义 `switch_mode` 绑定（默认 ctrl+t）恢复可用；busy flash / 任何中性提示不再有被误染 plan 色的风险。
- 不影响：turn 启动 / 退出 / Prompt / EditPlan / ReloadConfig 等非幂等命令仍走阻塞 `.send().await`（保证不丢）；slash 路径与 web API 的切换语义不变；worker `SwitchAgent` 臂消费逻辑不变。
- 已知残留：`queued_skill_drain` 集成测试失败为并行批次在途改动所致（干净 HEAD 同样失败），非本批次引入。

## Related Docs

- [plan-switch-direction-aware-running-gate.md](plan-switch-direction-aware-running-gate.md)（双向拦截契约；本批次为其键路径补通道压力卫生 + 白名单补全）
- `agents/tui/index.md`
