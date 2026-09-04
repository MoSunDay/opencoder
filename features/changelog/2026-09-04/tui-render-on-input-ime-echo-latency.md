Commit: e6f9c19

# TUI 输入即帧：按键/IME 回显不再等待 fps 帧周期，门控延迟归零

## Context

用户反馈中文 IME 输入回显可感延迟（打字后字符上屏慢）。排查确认根因
不在终端协议（kitty/ghostty/tmux 均正常），而在 `app.rs` 主循环的
**两级渲染门控**：

```
if dirty && render_pending { render_frame(...); dirty = false; }
render_pending = false;   // 每轮迭代无条件清零
```

普通按键（含 IME 提交的 Char 事件）只置 `dirty = true`，不置
`render_pending`；后者的唯一常态来源是 `frame_ticker.tick()`，
周期 = `config.tui_frame_ms()` = `fps.unwrap_or(10)` → 默认
10 FPS = 100ms/帧。⇒ 按键回显必须等下一个 tick：平均 ~50ms、
最坏 ~100ms。中文 IME 一次提交多个字符、用户盯屏等确认，比英文
连打更可感。

**历史回归**：`2026-07-16/fps-scroll-resume-dangling-tooluse.md` 曾以
FRAME_MS 100→33 修复过同款"打字可感延迟"，帧率配置化重构把默认值
退回 10 FPS 后延迟随之回归。本次从根上解耦，不再依赖帧率数值。

已排除的次要因素：Esc 守卫 80ms（只影响 Esc 类）、POLL_TIMEOUT
150ms（仅空闲唤醒上限）、BODY_REFRESH_MS 333ms（只管流式正文缓存）、
crossterm UTF-8 部分序列缓冲（罕见且 poll 即时就绪）。

## Change Summary

- **纯函数**（`app_loop.rs`）：新增
  `input_event_prompts_frame(&Event) -> bool`——Key/Paste/Mouse/
  Resize → true，FocusGained/FocusLost → false。
- **主循环接线**（`app.rs` 输入事件臂顶部）：`dirty = true;` 处同步
  `if input_event_prompts_frame(&ev) { render_pending = true; }`。
  一次覆盖所有输入面：composer 打字、粘贴（IME 大段提交）、plan
  编辑器、notepad、菜单、copy_mode、mouse、resize。分支内既有的
  `render_pending = true`（copy_mode/keymap/quit 等）变冗余但无害。
- 默认 fps=10 不动：帧率只管动画/流式正文节奏，与输入回显正交，
  idle CPU 不变（无输入时仍按 tick 渲染）。
- 成本论证：`compute_display` 本来每轮迭代都执行（与是否渲染无关）；
  `render_frame` 是 ratatui diff 渲染，无状态变化的事件 diff 为空、
  flush 零字节。"每键一帧"是 vim/helix 等 TUI 的标准做法；IME 提交
  N 字符 = N 个小 diff 帧，channel 背压（容量 256）不变。

## Impact Surface

- 仅 `crates/tui`：`app_loop.rs`（+函数）、`app.rs`（+3 行接线）；
  无接口变更、零终端转义序列变更。
- 多终端协议兼容性：修复点在应用侧渲染调度，kitty/ghostty/wezterm/
  foot/alacritty/Terminal.app/tmux/screen 字节流不变；f619bd5 的
  IME 修复及回归测试 `kitty_flags_keep_text_keys_off_csi_u_for_ime`
  继续守卫。tmux：输入为 legacy 编码即时转发、无输入侧批处理，
  ratatui diff 单次 write 即时 flush；resize 事件同受益（tmux 差分
  契约 2026-08-10 不受影响）。

## 测试清单（rules/01、02、03）

- 新增单测 `crates/tui/src/app_loop_render_prompt_tests.rs`（表驱动）：
  - `all_input_surfaces_prompt_an_immediate_frame`：ASCII Char、CJK
    Char（IME 提交）、Paste（多行中文）、Mouse（滚轮）、Resize →
    全部 true；
  - `focus_events_do_not_prompt_frames`：FocusGained/FocusLost →
    false（锁定"非输入面不触发帧"语义，防后人误删）。
- 回归：`cargo test -p opencoder-tui` 全量 1685 passed（含新增 2）；
  `cargo test --workspace` 全量通过；`cargo clippy -p opencoder-tui
  --all-targets` 新代码零告警；我改动的三个文件 `cargo fmt -- --check` 无 diff（crate 内 `chat_tests/*`、`worker/tests.rs` 存在 HEAD 遗留的未格式化代码，与本次无关，未触碰）。

## Related Docs

- agents/tui/index.md（主流程：渲染门控 `dirty && render_pending` 与
  输入即帧契约）
