Commit: (working-tree, 待提交)

# TUI 正常退出吸收在途 Kitty 释放报告：消除退出后 shell 首键乱码

## Context

push/pop 顺序修复（cfb8247）后，正常退出（无 tmux）仍偶发 shell 回显
`$ 442;1:3u0;1:3u` 之类乱码：crossterm 的 `DisableMouseCapture`/
`PopKeyboardEnhancementFlags` 写出后，终端里**在途的按键释放报告**
（`CSI <cp>;<mods>:<event> u`，来自按住 Enter/修饰键离开 TUI 的瞬间）
不被协议 pop 抵消，落到 shell 的 readline，被解释成插入文本 + 乱码。
分类器（shellguard）不涉入——这是纯终端 I/O 时序问题。

## Change Summary

- **quiesce 输入通道**（`terminal.rs`）：新增
  `write_quiesce_input()`——退出时先把键盘增强标志降到
  `DISAMBIGUATE_ESCAPE_CODES`（保留键位正确性、关闭逐键/事件上报），
  单缓冲写出，作为 `write_restore` 的前置步骤；原有
  `quiesce_input_reporting` 逻辑抽出来与 `write_restore` 复用同一序列。
- **drain 语义**（`input.rs`）：新增 `drain_shutdown()` 与
  `drain_until_quiet(quiet_ms=80, cap_ms=300)`——静默窗判定「无新字节
  即 quiet」，cap 兜底防慢终端卡死退出；只读事件队列，不注入任何键。
- **主循环接缝**（`app.rs`）：主循环 break（正常退出，无 tmux 分支）
  后调用 `drain_shutdown`，再走既有 restore。tmux 不透传协议，路径不变。

## Impact Surface

- 仅 `crates/tui`：`terminal.rs` / `input.rs` / `app.rs`；无接口变更。
- 新增 e2e：`tests/tui_exit_restore_e2e.rs::normal_quit_absorbs_kitty_release_reports`——
  正常退出后断言无 `CSI u` 释放报告泄漏到外层 shell。

## Notes / Compatibility

- 与 tui-kitty-keyboard-flags-leak（push/pop 备屏顺序）互补：那次修
  「进/出平衡」，这次修「退出时序吸收」；两者共同保证任何终端栈语义下
  退出后 shell 干净。
- 已被污染的会话不受追溯 healing，需 `reset`。

## Related Docs

- agents/tui/index.md（终端生命周期：quiesce/drain 不变量）
