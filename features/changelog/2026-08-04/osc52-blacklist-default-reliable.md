Commit: (working-tree, pre-initial-commit)

# feat(tui): OSC52 剪贴板可靠性模型从白名单翻转为黑名单（默认可靠）

## 背景

OSC 52 剪贴板转义序列被绝大多数现代终端支持（xterm、iTerm2、WezTerm、Alacritty、
kitty、foot、Windows Terminal 等），仅 VTE 系终端（GNOME Terminal / Terminator /
XFCE / MATE 等）与 GNU screen 会静默丢弃该序列。

旧模型采用**白名单**：只有命中 `RELIABLE_HINTS` 的已知终端才判为可靠，未识别终端
一律视为不可靠。这导致大量 SSH 远端会话（`TERM=xterm-256color`）在拖选复制时显示
`⚠ Copy unreliable`，即便实际终端完全支持 OSC52。

## 变更

### clip_probe.rs — 黑名单模型

- `osc52_is_reliable(is_vte, is_screen)` 翻转为 `!(is_vte || is_screen)`：默认
  可靠，仅排除 VTE 系与 GNU screen。
- 移除 `RELIABLE_HINTS` 白名单数组与 `wt_session` 白名单逻辑。
- 新增 `VTE_HINTS` 黑名单数组，供 `is_vte_terminal()` 识别 VTE 系终端。
- `ClipProbe.osc52_reliable` 字段文档更新为「true by default, false only for
  known stragglers」。
- 测试 `unknown_terminal_is_reliable_by_default` 锁定语义翻转（断言
  `osc52_reliable == true`）。

### selection.rs — SSH 提示文案 + 过期注释

- `status_message` 中 `osc52_reliable == false && ssh` 分支的提示文案更新，仅在
  真正不可靠时显示 `⚠ Copy unreliable — terminal may not support OSC52 —
  install xclip/xsel locally`。
- 修正 `finish_copy_returns_report_for_drag` 测试内的过期注释：
  「conservatively false for unidentified terminals」→「reliable by default for
  unknown terminals」（与黑名单模型一致）。

## 用户可感知变化

- SSH `xterm-256color` 终端拖选后现显示 `📋 Copied via OSC52`（而非
  `⚠ Copy unreliable`）。
- 仅 VTE 系 / GNU screen 终端仍显示不可靠提示。

## 测试覆盖

| 闸门 | 结果 |
|------|------|
| `cargo test -p opencoder-tui --lib clip_probe` | 21 passed; 0 failed |
| `cargo test -p opencoder-tui --lib selection` | 19 passed; 0 failed |
| `cargo test --workspace` | 1769 passed; 0 failed |
| `cargo clippy --workspace --all-targets -D warnings` | 0 warnings |
| `cargo build --workspace` | Finished |

关键测试：
- `unknown_terminal_is_reliable_by_default` — 锁定黑名单默认可靠语义（断言具体值
  `osc52_reliable == true` / `!is_vte`）。
- `reliable_windows_terminal_via_wt_session` — `WT_SESSION` 不再走白名单分支。
- `headless_detection` — headless 环境仍判 OSC52 可用。
- `copy_report_status_unreliable_with_ssh_hint` — 断言 ⚠ + "OSC52" 子串（SSH +
  不可靠路径）。

防修绿扫描：OSC52 文件无删除的 `#[test]`、无新增 `#[ignore]`、无
`assert!(true)`/`is_ok()`/`is_some()` 弱断言；测试函数净增（1 改名 + 新增
`unknown_terminal_is_reliable_by_default`），断言均为可观测值。

## 兼容性

- `ClipProbe` / `probe_clipboard` / `copy_local_smart` / `status_message` 公开
  签名未变，无外部断裂。
- `osc52_is_reliable` / `classify_terminal` / `RELIABLE_HINTS` 均为 clip_probe
  内部私有，无跨 crate 调用方。

## 备注

- 工作树在本会话内被外部多次改写，OSC52 主逻辑（clip_probe.rs / selection.rs）
  已随 `632e3d1` 提交入 HEAD；本条 changelog 补记其语义翻转与测试覆盖。
