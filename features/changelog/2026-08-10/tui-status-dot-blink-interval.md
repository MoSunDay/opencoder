Commit: 0e0ec867c45170ffb244e38469baf7f4508bacc9

# TUI 状态圆点闪烁节奏放缓

## Context

running 状态圆点原先随 100ms 动画 tick 每 tick 翻转，实际为 100ms 亮 / 100ms 灭，完整周期 200ms，视觉上过于急促。原变更记录中的“200ms 亮 / 200ms 灭”与实现也不一致。

## Change Summary

- 保持共享 100ms 动画 tick 与 spinner 速度不变，只把状态圆点每个明灭相位扩展为 5 tick。
- running 时采用 500ms 亮 / 500ms 灭（完整周期 1s）；idle 时继续常亮。
- 灭相位继续使用两个等宽空格，mode chip 不发生水平位移。
- 用相位末端 tick 4 和切换边界 tick 5 的渲染测试锁定间隔。

## Impact Surface

仅改变 TUI 底部状态栏模式圆点的闪烁节奏。spinner、帧率配置、事件循环、headless CLI 与持久化格式不变。

## Validation

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 500ms 亮相位与下一周期边界 | `status_dot_stays_visible_through_first_phase` | `render_tests/status_bar.rs` |
| 500ms 灭相位且 mode chip 不抖动 | `status_dot_stays_hidden_through_second_phase` | `render_tests/status_bar.rs` |
| idle 圆点常亮 | `status_dot_stays_steady_when_idle` | `render_tests/status_bar.rs` |

- `cargo test --workspace --quiet`：2308 passed / 0 failed / 0 ignored。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo build --workspace`：成功。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
