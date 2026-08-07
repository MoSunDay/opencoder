# TUI 三连修：启动残影 / tmux 复制失效 / Esc 拆分乱码

## 背景
tmux 下 TUI 有三个长期观感问题：
1. **残影**：退出后再启动，上一轮渲染的字符残留在屏幕上（tmux 终端持久化 + 启动未清屏）。
2. **tmux 复制失效**：`osc52_is_reliable` 在 tmux 下判定不可靠 → `copy_to_clipboard` 无可用
   后端 → 复制静默失败；tmux `set-clipboard` 状态未被探测。
3. **Esc 乱码**：tmux escape-time（默认 500ms）会把按下的 Esc 与后续按键合并成
   Alt+char / CSI / SS3 序列；按方向键会插入 `[A`/`[D` 等垃圾文本。

## 变更

### Fix 1 — 启动残影（`app_bootstrap.rs` + `render_clear_tests.rs`）
- `Term::new(backend)?` 之后追加 `terminal.clear()?;`（ESC[2J + 重置 back buffer），
  清除上一轮 tmux 持久化的字形。附 tmux-persistence 注释说明。

### Fix 2 — tmux 剪贴板（`clip_probe.rs` + `selection.rs`）
- `clip_probe.rs` 新增 `tmux_clipboard: Option<bool>` 字段；`tmux_set_clipboard_status()`
  I/O 封装（`tmux show-options -g set-clipboard`）；纯函数 `parse_tmux_clipboard`
  （on/external→Some(true)，off→Some(false)，其余→None）。
- `osc52_is_reliable` 签名改为 `(is_vte, is_screen, is_tmux, tmux_clipboard)`：tmux 下
  fail-closed，仅当探测到 `Some(true)`（set-clipboard on/external）才认为可靠。
- `classify_terminal` 保持纯函数（tmux 时传 None）；`probe_clipboard` 填充该字段并重推导
  最终判定。
- `selection.rs`：`CopyReport` 新增 `tmux_buffer: bool`；当本地工具不可用 && tmux &&
  OSC52 不可靠时，`copy_to_clipboard` 走 `try_spawn("tmux", ["load-buffer", "-"], text)`
  写入 tmux buffer。**故意不带 `-b NAME`**：tmux 默认粘贴（`paste-buffer`，即
  `prefix ]`）只定位最近添加的自动命名 buffer（`paste_get_top` 跳过显式命名 buffer），
  命名加载（如 `-b opencoder`）将无法用 `prefix ]` 粘出——review 实测 tmux 3.6 源码
  （paste.c `paste_get_top` / cmd-load-buffer.c `paste_set`）确认后修正。
  `status_message` 增加 tmux-buffer 分支并更新提示文案。

### Fix 3 — Esc 拆分乱码（`key_handler.rs` + `input.rs`）
- `key_handler.rs`：`KeyCode::Char(c)` 分支顶部新增守卫——`ALT` 修饰下的字符一律
  `KeyAction::None`（不插入文本）。显式 Alt 绑定（f/F/b/B/Tab）在上方分支保持原语义，
  Alt+Ctrl 组合不受影响。
- `input.rs`：新增 `EscGuard` 状态机（`EscGuardState {Idle, Holding, SwallowTail}`，
  `ESC_GUARD_WINDOW = 80ms`）：
  - `esc_guard_feed(state, expired, ev)` 纯函数转移：单次 Esc 触发 Holding 并暂存；
    窗口内再次收到 Esc 视为双击放行两枚 Esc；窗口内收到普通按键放行 Esc + 该键；
    窗口内收到 CSI/SS3 起始字节（`[`/`O`）→ SwallowTail 吞掉后续序列（直到放行/超时）。
  - `EscGuard` 运行时结构体（`poll_timeout` / `flush_expired` / `feed`）接入
    `spawn_input_pump` 事件循环，超时回退 `read_char_timeout` 用 `poll_timeout` 缩短窗口。
- 方向键 / 插入键 / 功能键的拆分序列在 tmux escape-time 下不再泄漏为垃圾文本。

## 测试覆盖（当次实跑）
- `cargo test --workspace` → **1933 passed; 0 failed**（2aaa1c4 实跑 1932，review 修正
  tmux 回退后 +1；含并发 SubagentSteer 功能测试）。
- `cargo test -p opencoder-tui` → 985 passed; 0 failed。
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告。

### 新增测试清单（16 个）
| 测试 | 文件 | 说明 |
| --- | --- | --- |
| `startup_clear_wipes_glyphs_persisted_by_previous_run` | `render_clear_tests.rs` | 复用 swap_buffers + reset 建模上次运行残留，断言 `clear()` 清空（Fix 1） |
| `parse_tmux_clipboard_values` | `clip_probe.rs` | on/external→true、off→false、空/未知→None（Fix 2） |
| `osc52_reliable_respects_tmux_clipboard` | `clip_probe.rs` | tmux + None/off → false（fail closed）；tmux + on → true（Fix 2） |
| `classify_tmux_fails_closed_until_probed` | `clip_probe.rs` | 探测前 classify 不宣称 OSC52 可靠，探测后重推导（Fix 2） |
| `copy_report_status_with_tmux_buffer` | `selection.rs` | CopyReport.tmux_buffer 状态文案分支（Fix 2） |
| `tmux_fallback_loads_automatic_buffer_not_named` | `selection.rs` | 回退必须 `load-buffer -`（自动命名），`prefix ]` 才可粘贴（Fix 2 回归） |
| `handle_key_alt_char_is_dropped_not_inserted` | `key_handler_tests.rs` | Alt+普通字符被吞，不插入输入框（Fix 3） |
| `handle_key_alt_f_still_moves_word` | `key_handler_tests.rs` | Alt+F 显式绑定仍生效（Fix 3 回归守卫） |
| `esc_guard_single_esc_held_then_committed_on_expiry` | `input.rs` | 单 Esc 窗口内不放行，过期后提交（Fix 3） |
| `esc_guard_double_esc_passes_both` | `input.rs` | 窗口内连续两次 Esc 双放行（Fix 3） |
| `esc_guard_esc_then_normal_key_passes_both` | `input.rs` | Esc + 普通键放行两者（Fix 3） |
| `esc_guard_swallows_split_csi_arrow` | `input.rs` | Esc + `[` + 方向键序列整体吞掉（Fix 3） |
| `esc_guard_swallows_split_ss3` | `input.rs` | Esc + `O` SS3 序列整体吞掉（Fix 3） |
| `esc_guard_swallows_split_insert_key` | `input.rs` | Esc + `[2~` 插入键序列整体吞掉（Fix 3） |
| `esc_guard_residue_stops_at_window_expiry` | `input.rs` | 窗口过期后残留状态复位、后续按键正常（Fix 3） |
| `esc_guard_non_key_event_after_esc_flushes` | `input.rs` | Esc 后非按键事件立即冲刷暂存 Esc（Fix 3） |

### 存量回归
- `cargo test --workspace` 全绿，无 `#[ignore]` / 删测试 / 弱断言。
