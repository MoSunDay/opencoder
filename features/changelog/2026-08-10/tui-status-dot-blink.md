Commit: 1648be8893e002febcee6ebfb82b3226c9ace4c5

# 左下角模式圆点 running 时闪烁

## Context

TUI 底部状态栏最左侧的 `●` 圆点当前按 mode 取色（plan=黄、act=青），但运行中与空闲没有任何区分，用户无法仅凭圆点判断任务是否在跑。

## Change Summary

- `crates/tui/src/render_status.rs` 提取纯函数 `status_dot(running, anim_tick, mode) -> Span`。
- 非 running：`● ` 常亮，颜色仍由 mode 决定（不闪）。
- running 且 `anim_tick % 2 == 0`（亮帧）：`● ` 正常渲染。
- running 且 `anim_tick % 2 == 1`（灭帧）：渲染两个等宽空格替换 `"● "`，列宽不变，`[act]`/`[plan]` 芯片不左右抖动。
- 复用既有 `anim_tick`（100ms 步进、仅 running 时推进）与 10FPS 重绘路径，无新计时器、无调用链改动、无目录结构调整。闪烁周期 ≈ 200ms 亮 / 200ms 灭。

## Impact Surface

只改变 TUI 状态栏圆点的渲染；headless CLI、事件、配置、持久化格式均不变。空闲帧与原先逐像素一致。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| running + 亮帧显示圆点 | `render::tests::status_bar::status_dot_shows_on_running_even_frame` | `crates/tui/src/render_tests/status_bar.rs` |
| running + 灭帧隐藏圆点且芯片列稳定 | `render::tests::status_bar::status_dot_hides_on_running_odd_frame` | `crates/tui/src/render_tests/status_bar.rs` |
| 空闲任意 tick 圆点常亮 | `render::tests::status_bar::status_dot_stays_steady_when_idle` | `crates/tui/src/render_tests/status_bar.rs` |

- 全量回归：`cargo test --workspace` → 2299 passed / 0 failed（首轮 2 个 bash 用例偶发失败，属真实 bash 子进程并行资源竞争，单独重跑通过，与本次改动无关）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
- [Features 索引](../../../features/index.md)
