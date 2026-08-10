Commit: b88f82225eae540d672b01c133234cdf60e26a2d

# TUI 在 tmux 中按帧完整重绘

## Context

启动清屏、resize 清屏、动态文本净化和同步帧已消除 Linux 直接终端的残影，但 Mac IDE/iTerm2 经 SSH 连接远端 Linux tmux 时仍可能在运行期留下旧字符。残影出现后 Ctrl+F 可立即清除，说明 Session 与渲染数据正确，偏差位于 ratatui diff baseline、tmux pane grid 和外层终端之间。

## Change Summary

- 新增纯函数式 `FrameRefreshPolicy`：非 tmux 保留增量 diff，tmux 使用完整重绘。
- tmux 每个实际提交帧按 synchronized begin → `Terminal::clear()` → draw → synchronized end 执行；清屏同时清理物理 grid 并重置 inactive diff buffer。
- prepare 或 draw 失败仍发送 end，避免终端停留在同步模式；错误继续向上报告。
- tmux 策略通过既有 `TMUX` 进程环境判定并缓存，不增加配置、环境变量或公开接口。

## Impact Surface

- Mac/iTerm2、IDE 终端或其他客户端通过 SSH 使用远端 tmux 时，新帧不再叠加运行期旧字符。
- tmux 以额外帧输出带宽换取显示正确性；非 tmux 性能路径不变。
- Session、Store、CLI、Web、数据库和模型输出均不变。

## Validation

- 帧策略、完整重绘、非 tmux no-op、同步顺序以及 prepare/render/end 失败路径均有单元测试。
- `cargo test -p opencoder-tui`：1185 passed，0 failed。
- `cargo test --workspace`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo build --release`：通过。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
