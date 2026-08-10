Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# TUI 使用同步帧消除 tmux 流式残影

## Context

既有启动清屏和 resize 清屏能消除持久 alternate-screen 与尺寸变化留下的旧字符，
ratatui 自身也会在每次 draw 后重置下一侧 diff buffer；但一次 `terminal.draw` 仍会分批写入终端。tmux 或外层终端若在
写入中途刷新，会短暂同时展示上一帧字符和未完成的新帧，形成无需用户操作也可能出现的
概率性叠字残影。

## Change Summary

- 主 TUI 帧以 CSI `?2026` synchronized-update begin/end 序列包裹，整帧完成后再展示。
- begin 仅 `queue!`，复用 `Terminal::draw` 的既有 flush；end 才主动 flush，每帧只新增
  一次 flush 和 16 字节控制序列，正常路径无堆分配。
- draw 失败时仍发送 end，避免终端停留在同步模式；begin、draw、end 错误均向上返回。
- 保留启动和 resize 清屏；移除冗余的全屏逐帧 `Clear`，避免热路径额外执行一次
  O(viewport cells) buffer 遍历。未增加配置、环境变量或外部接口。
- 不支持 synchronized-update 的终端按 ANSI 私有模式规则忽略序列，维持原渲染行为。

## Impact Surface

- tmux 中模型流式输出、spinner 和状态信息刷新不再暴露半写入帧。
- 非 tmux 且支持 synchronized-update 的终端同样获得原子帧展示。
- Session、Store、CLI、Web 和渲染数据模型均不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| begin/frame/end 顺序与 flush 数 | `synchronized_frame_wraps_output_in_mode_2026` | `tui/src/frame.rs` |
| render 失败仍发送 end | `synchronized_frame_always_ends_after_render_error` | `tui/src/frame.rs` |
| end 失败向上报告 | `synchronized_frame_reports_end_failure` | `tui/src/frame.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- UI/终端定向：3 项同步帧测试验证 CSI `?2026` 字节顺序、flush 次数以及 render/end 错误清理；不支持该私有模式的终端按既有 ANSI 兼容路径忽略序列。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
