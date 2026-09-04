Commit: e899495

# TUI 退出后 Kitty 键盘增强泄漏：部分终端逐键 CSI u 乱码、按键失效

## Context

退出 opencoder 后，在未开 tmux 的部分终端里 shell 提示符回显 `0;5:3u` 之类乱码，
且所有按键「失效」——实际是每个按键都以 kitty keyboard protocol 的
`CSI <cp>;<mods>:<event> u` 序列上报，readline 无法解释。

## Change Summary

- 根因：`TerminalGuard::enter()` 在 `EnterAlternateScreen` **之前**
  `PushKeyboardEnhancementFlags`（落在主屏栈），`restore()` 在备屏内 pop。
  协议规定增强标志栈按 screen 维护：进备屏保存主屏栈、出备屏丢弃备屏栈并
  恢复主屏栈——push 从未被抵消，退出后 `REPORT_ALL_KEYS_AS_ESCAPE_CODES`
  仍在主屏生效。
- 「部分终端」的分布与此完全吻合：per-screen 栈实现（kitty/ghostty/
  新版 wezterm/foot）泄漏；全局栈实现不泄漏；tmux 不透传 kitty 协议时不可见。
- 修复（`crates/tui/src/terminal.rs`）：push 移到进入备屏之后、pop 保持
  在离开备屏之前，push/pop 严格包进备屏会话——在两种栈语义下均平衡
  （helix/lazygit 同款顺序）；enter 序列抽成 `write_enter` 与既有
  `write_restore` 对称，且单缓冲一次写出，防止交错撕裂。

## Impact Surface

- 仅 `crates/tui/src/terminal.rs`：`TerminalGuard::enter`/`restore` 与
  `write_enter`/`kitty_enhancement_flags`；无接口变更。
- 新增回归测试：`write_enter_pushes_kitty_only_inside_alt_screen`、
  `write_restore_pops_kitty_before_leaving_alt_screen`（进/出双向顺序不变量）。

## Notes / Compatibility

- 已被旧版本泄漏污染的终端会话不受本次修复追溯 healing，需 `reset` 或重开
  终端；新会话起进入/退出平衡。
- 全量回归：`cargo test --workspace` 253 套件 / 3999 通过 / 0 失败。

## Related Docs

- agents/tui/index.md（终端生命周期条目补顺序不变量）
