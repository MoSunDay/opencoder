Commit: af71944

# TUI 无法输入中文（IME）：移除 REPORT_ALL_KEYS_AS_ESCAPE_CODES，文本键回归 UTF-8 通道

## Context

裸终端（未开 tmux）里 TUI 无法输入中文/日文等 IME 文字；在 tmux 内一切正常。
两类环境唯一的协议差异：tmux 不透传 Kitty 键盘增强 push，应用收到的始终是
legacy 编码（IME 提交文本为原生 UTF-8）；原生支持 kitty keyboard protocol 的
终端（kitty、ghostty、新版 wezterm/foot、alacritty 0.13+）则真实启用推送的标志。

## Change Summary

- 根因：`kitty_enhancement_flags()` 推送了 `REPORT_ALL_KEYS_AS_ESCAPE_CODES`。
  该标志把**文本键也改为 `CSI unicode;mods;text u` 上报**，IME 提交的文字只能
  经第三段（text-as-codepoints）到达，而 crossterm 0.28 的
  `parse_csi_u_encoded_key_code` 从不解析第三段——中文在应用侧静默丢失。
  tmux 内 push 被吞、始终走 legacy UTF-8 通道，掩盖了问题，形成「tmux 内正常、
  裸终端坏」的分布。
- 修复（`crates/tui/src/terminal.rs::kitty_enhancement_flags`）：永久移除
  `REPORT_ALL_KEYS_AS_ESCAPE_CODES`，保留 `DISAMBIGUATE_ESCAPE_CODES`（Esc/
  Ctrl 组合消歧，不影响文本通道）、`REPORT_ALTERNATE_KEYS`（shift 和弦备选）、
  `REPORT_EVENT_TYPES`（release/修饰键事件，release 由
  `consume_modifier_or_release` 过滤）。三者均不触碰文本通道，规范下文本键
  继续以原生 UTF-8 送达，全终端 IME 恢复。
- 多终端兼容核对：协议不支持的终端（tmux/screen/Terminal.app/Linux console/
  Windows conhost）本就忽略 push，legacy 路径字节级不变；仅支持部分标志的
  终端按规范忽略未支持位。Shift+Tab 走 legacy `CSI Z` → crossterm `BackTab`
  → keymap 归一化（`Tab+SHIFT`），无 ALL_KEYS 依赖；退出路径 release 报告的
  quiesce/drain 行为不变。

## Impact Surface

- 仅 `crates/tui/src/terminal.rs`（`kitty_enhancement_flags` + 相关注释）与
  两处注释措辞（`key_handler.rs`、`key_handler_running_mode_tests.rs` 的
  (Tab, SHIFT) 和弦拼写说明）；无接口变更。
- 另修复工作树内三处存量测试破损（与并行渲染重构的语义对齐）：
  `drain_discards_in_flight_events_then_stops_on_quiet` 保持 sender 存活以
  真正验证静默窗路径（channel-closed 短路由既有
  `drain_returns_promptly_when_channel_closed` 单测）；`1 Step` 单复数期望
  （0c2de6c 渲染行为）；StepGroup 自带尾随空行后 Done 不再堆叠边界 Marker。
  并修复工作树中一段错乱重复的 `hook_body_restores_before_chaining_to_prev`
  测试片段（上轮遗留）。

## 测试清单

- 新增回归守卫：`terminal::tests::kitty_flags_keep_text_keys_off_csi_u_for_ime`
  （flags 不含 `REPORT_ALL_KEYS_AS_ESCAPE_CODES`、保留 `DISAMBIGUATE_ESCAPE_CODES`）。
- 更新：`input::tests::drain_discards_in_flight_events_then_stops_on_quiet`、
  `chat_tests::tool_output_blank::collapsed_group_shape_is_unchanged`、
  `chat_tests::thinking_state::completed_answer_creates_say_when_every_text_delta_was_dropped`。
- TUI 单测：1660 通过 / 0 失败；e2e `tui_exit_restore_e2e` 3/3；
  全量回归 `cargo test --workspace` 253 套件 / 4013 通过 / 0 失败。

## Notes / Compatibility

- 已开启 TUI 的会话退出后自动 pop，无需用户动作；被旧版泄漏污染的终端不受
  追溯影响。中文输入在 kitty/ghostty/wezterm/foot/alacritty/Terminal.app/tmux
  全部恢复 legacy 文本通道语义。
- copy-mode 的 Shift-suspend 依赖 `REPORT_EVENT_TYPES`（保留），行为不变。

## Related Docs

- agents/tui/index.md（键盘增强标志集合与 IME/UTF-8 通道语义）
- [tui-kitty-keyboard-flags-leak.md](tui-kitty-keyboard-flags-leak.md)（push/pop 备屏顺序不变量）
