Commit: (working-tree, steer 点击 `>` stale 焦点回退父路径立即打断——显示/点击路由统一存活判定)

# TUI steer 点击 `>` 失效焦点修复：点击即打断父回合

## 背景

用户报告：steer 行点击 `>` 没有立刻打断并提交（表现为无操作）。根因在 TUI
点击路由，而非 runner——

- `resolve()`（`crates/tui/src/steer_fire.rs`）以 `subagent_focus.is_some()`
  判定走子代理路径，不区分焦点是否存活；
- `fire_subagent_turn_cancel` 对 done/越界焦点静默 `return`，无任何动作；
- 显示层 `steer_queue_sources`（`crates/tui/src/app_display.rs`）对同样焦点
  却**回退父行**显示。

结果：面板显示父 steer 行、点击却路由到已死的子 token——「显示」与「动作」
分叉，点击表现为静默无操作。runner 侧契约（turn_cancel → 立即打断 → 下个
边界吸收 steer）经 session 集成测试证实本来正确，未改动。

## 实现

- 新增 `subagent_input::is_live_subagent_focus`：唯一存活判定。
- `steer_fire.rs::resolve`：live 焦点 → 只打子 token（原语义钉死）；
  stale/越界焦点 → 回退父路径 `fire_turn_cancel`，立即打断父回合，steer 于
  下个边界被吸收。
- `app_display.rs`：`steer_queue_sources` 与 `is_input_disabled` 改用同一
  判定——面板显示与点击路由永不分叉（消除分叉类缺陷，非补丁式特判）。
- 附带编译前置：sidecar 线新增 `SessionEvent::Sidecar*` 枚举使 TUI 编译
  失败，`chat.rs::apply` 补最小 arms（归属 sidecar 线条目，此处仅记录前置）。

## 全局影响

仅 `crates/tui`；被改函数均为 crate 私有，无外部消费者。无公开 API、持久化、
配置形状变更；`SteerConsumed` 事件流与 store 不变量未触碰。web 端 steer 在
准入时直接打父 `turn_cancel`（`crates/web/src/handle.rs`），无子代理焦点
概念，本缺陷类不适用（已核实）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| done 焦点回退父 steer | `done_subagent_focus_falls_back_to_parent_steer` | `crates/tui/src/steer_fire.rs` |
| 越界焦点回退父 steer | `stale_focus_index_falls_back_to_parent_steer` | `crates/tui/src/steer_fire.rs` |
| live 焦点仅打子 token | `live_subagent_focus_targets_child_token_only` | `crates/tui/src/steer_fire.rs` |
| runner 契约：打断后吸收 | `parent_turn_cancel_steer` | `crates/session/tests/parent_turn_cancel_steer.rs` |
| runner 契约：终态收束 | `parent_steer_terminal` | `crates/session/tests/parent_steer_terminal.rs` |
| 子代理 steer 端到端 | `subagent_steer` | `crates/session/tests/subagent_steer.rs` |
| 裸 steer 短路 | `bare_steer_short_circuit` | `crates/session/tests/bare_steer_short_circuit.rs` |

## 回归

- steer_fire 过滤复跑（当前树 bin `opencoder_tui-bd84d72321403751`）：
  16 passed / 0 failed（1532 filtered，合计 1548 与全量口径一致）
- runner 契约集成 bin 新鲜复跑：`parent_turn_cancel_steer` 1/0、
  `parent_steer_terminal` 2/0、`subagent_steer` 3/0、
  `bare_steer_short_circuit` 2/0
- 全量回归：`cargo test --workspace --no-fail-fast` → 待补（收敛树上的
  gate 运行，数字以实际输出为准）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 待补

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [features/index](../../index.md)
