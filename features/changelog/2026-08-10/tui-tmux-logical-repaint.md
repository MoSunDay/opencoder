Commit: 911ef830e966d132170f2798001d452ee8776849

# tmux 完整重绘不再逐帧物理清屏

## Context

tmux 每帧执行 `Terminal::clear()` 虽能清除 Mac/iTerm2 远程链路中的旧残影，但 ESC[2J 在同步更新未被整条链路原子呈现时会暴露空白帧，形成持续闪屏。稳态刷新需要覆盖失配的 pane grid，而不是反复清空物理终端。

## Change Summary

- tmux 帧只在内存中废弃 ratatui inactive diff baseline，强制下一次 draw 输出全部普通单元格，包括用于擦除旧内容的空白。
- prepare 阶段不再调用 `Terminal::clear()`，物理 grid 保持上一完整帧直到新帧覆盖。
- 非 tmux 增量路径以及启动、resume、resize 生命周期的单次 clear 保持不变。
- 保留 synchronized begin/end 和失败后的 end 清理语义。

## Impact Surface

- Mac IDE/iTerm2 经 SSH 使用远端 tmux 时，持续输出同时避免旧残影与逐帧闪屏。
- tmux 仍承担完整帧输出带宽；直接终端性能不变。
- 不增加配置、环境变量、公开接口或数据变更。

## Validation

- tmux 完整逻辑重绘覆盖相同帧、旧字符到空白、零物理 clear；非 tmux 保持增量路径。
- `cargo test -p opencoder-tui`、workspace tests、workspace clippy 与 release build。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
