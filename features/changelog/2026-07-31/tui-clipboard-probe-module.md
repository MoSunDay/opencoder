Commit: (working-tree, pre-initial-commit)

# TUI 剪贴板探测模块——终端能力分类 + 诚实的复制反馈

## 背景

旧版 `selection.rs` 的剪贴板复制逻辑有三个问题：
1. **OSC52 盲目乐观**——`CopyReport.osc52` 恒为 `true`，即使运行在 VTE 终端
   （GNOME Terminal / Terminator / Xfce / MATE）或 GNU screen 中，这些终端
   默认静默丢弃 OSC52 序列，用户看到"已复制"却无法粘贴。
2. **本地命令不区分环境**——在 SSH 会话中执行 `xclip` 会写入 *远程* 机器的
   剪贴板，用户完全看不到；在无显示服务器的 headless 环境也会无谓尝试。
3. **逻辑耦合在 selection.rs**——~120 行平台特定代码与选择渲染混在一起，
   无法独立测试终端分类逻辑。

## 变更

### 新增 `crates/tui/src/clip_probe.rs`
- **`ClipProbe` 结构体**：终端/显示环境快照——`is_vte`、`is_screen`、
  `is_tmux`、`is_ssh`、`osc52_reliable`、`wayland`、`x11`。纯数据，无方法。
- **`classify_terminal(get_var)`**：纯函数，接收 env-var 访问闭包，完全可
  测试。检测 VTE 指纹（`TERM_PROGRAM`/`COLORTERM`/`TERM`）、可靠终端白名单
  （iTerm2/WezTerm/Alacritty/kitty/ghostty/foot/Windows Terminal）、tmux/
  screen/SSH/Wayland/X11。
- **`probe_clipboard()`**：探测真实环境，`OnceLock` 缓存——重复复制不重复探测。
- **`copy_local_smart(probe, text)`**：智能本地命令分发。SSH 跳过（写远端剪贴
  板无意义）；Wayland 优先 `wl-copy` 回退 `xclip`/`xsel`；headless 直接返回
  `None`。
- **`try_spawn`** + **`CLIP_CMD_TIMEOUT`**：从 selection.rs 原样迁入，3s 超时
  + kill，poll-wait 避免阻塞。
- 分类逻辑测试：VTE 检测（gnome/terminator/xfce）、可靠终端识别、
  Wayland/X11/headless 区分、SSH 跳过、try_spawn 行为。

### `crates/tui/src/selection.rs`
- **`CopyReport`** 字段重构：`osc52: bool` → `osc52_reliable: bool`（探测判决），
  新增 `tmux: bool`、`ssh: bool`（失败提示上下文）。
- **`status_message()`** 三态消息：
  - 本地工具成功 → 绿色 "Copied N line(s) (xclip)"
  - OSC52 可靠、无本地工具 → 绿色 "Copied N line(s) via OSC52"
  - 两者皆否 → 红色 "⚠ Copy unreliable" + 上下文提示
    （tmux: set-clipboard / ssh: Shift+drag / 通用: install xclip）
- **`copy_to_clipboard()`** 调用 `clip_probe::probe_clipboard()` +
  `copy_local_smart()`，从探测结果填充 `CopyReport`。
- 删除 `copy_local`/`try_spawn`/`CLIP_CMD_TIMEOUT`/`osc52_only_message`/
  `under_tmux`（全部迁入 clip_probe）。
- 测试更新：移除 `try_spawn` 测试（迁入 clip_probe），`CopyReport` 测试改为
  新字段，新增 tmux/ssh/通用三种失败提示测试。

### `crates/tui/src/lib.rs`
- 新增 `pub mod clip_probe;`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| VTE 终端检测 | `detects_vte_via_term_program` 等 | clip_probe.rs |
| 可靠终端白名单 | `detects_reliable_iterm` 等 | clip_probe.rs |
| Wayland/X11/headless | `wayland_detection`/`x11_detection`/`headless_detection` | clip_probe.rs |
| SSH 跳过本地命令 | `copy_local_skips_on_ssh` | clip_probe.rs |
| try_spawn 行为 | `try_spawn_*` (3 tests) | clip_probe.rs |
| 本地工具成功消息 | `copy_report_status_with_local_tool` | selection.rs |
| 可靠 OSC52 消息 | `copy_report_status_reliable_osc52_no_tool` | selection.rs |
| tmux 失败提示 | `copy_report_status_unreliable_with_tmux_hint` | selection.rs |
| ssh 失败提示 | `copy_report_status_unreliable_with_ssh_hint` | selection.rs |
| 通用失败提示 | `copy_report_status_unreliable_generic_hint` | selection.rs |

- 用户指示 **免测**，未运行 `cargo test --workspace`。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- build：`cargo build --workspace` → 编译干净。
- 行数：clip_probe.rs 397 ≤ 400；selection.rs 467 ≤ 800。

## Impact Surface
- 用户感知：在 VTE/screen/headless 终端中复制文本时，不再看到误导性的
  "已复制"，而是看到诚实的 "⚠ Copy unreliable" + 具体修复建议。
- OSC52 仍然总是发送（best-effort），只是消息反映了探测的真实置信度。
- SSH 会话不再无谓尝试远程剪贴板命令（节省 ~3s 超时等待）。
- 不影响：CLI/Web/session/store 边界；仅 TUI 鼠标选择的复制路径。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
