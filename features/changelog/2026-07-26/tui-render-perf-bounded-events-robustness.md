Commit: (working-tree, pre-initial-commit)

# perf(tui,session): 渲染视口虚拟化 + 有界事件缓冲 + TUI 健壮性修复

## 背景
TUI 的每帧渲染（spinner 动画 tick 每 ~300ms 触发）会对**整个 transcript** 调用
`ChatView::flatten()` + `Paragraph::new(lines)`，导致逐帧成本随会话长度 O(n) 增长——
长会话下动画卡顿、滚动迟滞。同时存在若干健壮性缺口：

1. **事件缓冲无界**：`spawn_event_flusher` 使用 `mpsc::unbounded_channel`，当 DB 落后
   时内存无上界（O(未刷写 tail)），存在长会话内存膨胀风险。
2. **stale cancel 标志**：一次 cancel 置位的 `cancelled` 标志未被清除，会抑制下一个 turn
   的启动。
3. **无 Ctrl+C 取消**：raw 模式下 Ctrl+C 是 ETX 字符而非 SIGINT，此前未映射到 cancel。
4. **poison lock**：`skill_handle` Mutex 在前次 panic 后 `unwrap()` 会再次 panic。
5. **worker 线程 panic 损坏终端**：worker panic 走默认 hook 打到 stderr，污染
   alternate-screen 显示。
6. **无界粘贴**：`insert_str` 无大小上限，粘贴巨型 blob 可无界增长内存。

## 变更

### A1 渲染视口虚拟化（per-frame O(visible_h)）
- **`crates/tui/src/render_viewport.rs`**（新文件，142 行）：`ViewportCache` 缓存 flatten
  后的 `Vec<Line>` + 累计行偏移表 `cum_rows`；`visible_window()` 二分查找可见窗口
  O(log n)，仅克隆可见行供 `Paragraph` 渲染。
- **`crates/tui/src/render.rs`**：`render_body` 仅在 cache 缺失/宽度变化时重建
  （`:318` `is_none_or`）；`:360` 切片可见窗口而非传整个 transcript；
  `MouseHits.total_rows`（`:65`）暴露给滚动轮用。
- **`crates/tui/src/app.rs:262`**：持有 `Option<ViewportCache>`，在 body-refresh 节奏
  处（`:296`）置 `None` 强制重建。
- **`crates/tui/src/app_helpers.rs:723`**：滚动轮改用 `hits.total_rows`，不再每次 wheel
  事件重新 flatten 整个 transcript（同时移除 `Paragraph/Wrap` import）。

### 有界事件缓冲（backpressure 安全）
- **`crates/session/src/event_sink.rs:36`**：`pub const CAPACITY = 4096`，unbounded →
  bounded channel；`push` 改 `try_send`——channel 满时 delta 片段静默丢弃（仅显示用，
  权威文本经 per-turn messages append 落库），结构化事件返回 `Full` 由调用方处理。
- **`crates/session/src/resume.rs:437`** / **`crates/session/src/runner/subagent.rs:186`**：
  `unbounded_channel` → `mpsc::channel(CAPACITY)`，`send` → `try_send`，delta 在
  backpressure 下静默丢弃、其余事件 log 后丢弃（与 `EventSink::push` 同语义）。

### TUI 健壮性修复
- **`crates/tui/src/app.rs:547`**（B3）：`running=true` 前清 `cancelled=false`，消除
  上一次 cancel 的残留标志抑制新 turn。
- **`crates/tui/src/key_handler.rs:173`**（B4）：`Ctrl+C`（CONTROL + `Char('c')`）映射
  到 `KeyAction::Cancel`（运行中时等效双击 Esc）。
- **`crates/tui/src/app.rs:640`**：`skill_handle` 的 `unwrap()` →
  `unwrap_or_else(|e| e.into_inner())`，poison lock 后仍可恢复。
- **`crates/tui/src/terminal.rs:66`**（C1）：捕获的 panic 发结构化
  `tracing::error!`。
- **`crates/tui/src/terminal.rs:96`**（C4）：worker 线程 panic 改为写入
  `tui-panic.log`（`write_panic_log`），不再走默认 hook 打 stderr 污染 alternate-screen。
- **`crates/tui/src/composer.rs:107`**（C3）：`MAX_INPUT_CHARS = 256KiB` 上限，超限
  粘贴静默拒绝（防无界内存）。

### scroll 类型拓宽 u16 → u32
- **`render.rs` / `frame.rs` / `app.rs` / `app_helpers.rs` / `key_handler.rs` /
  `selection.rs` / `session_ui.rs` / `app_task.rs`**（及对应测试）：`scroll`/`parent_scroll`
  由 `u16` 拓宽到 `u32`，支持 >65535 行的超长 transcript（此前长会话滚动溢出归零）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 视口缓存构建/切片 1k/5k/10k 块 | `viewport_build_and_slice_{1k,5k,10k}_blocks` | `crates/tui/tests/perf_long_session.rs` |
| per-frame 成本 O(visible_h) 非O(n) | `per_frame_cost_bounded_by_visible_h_not_block_count` | `crates/tui/tests/perf_long_session.rs` |
| 满通道下 delta 丢弃/结构事件返 Full | `push_drops_delta_but_surfaces_structural_on_full_channel` | `crates/session/src/event_sink.rs` |
| 超限粘贴被拒绝 | `insert_str_rejects_oversized_paste` | `crates/tui/src/composer.rs` |

- 全量回归：`cargo test --workspace` → 全绿（0 failed，79 个 test result 套件 ok）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零 warning
  （修复 `render.rs:318` `map_or(true,…)` → `is_none_or`）。
- 构建：`cargo build --workspace` → 干净零 error。
- 行数（新文件 ≤ 400，迭代中文件 ≤ 800）：
  - 新文件 `render_viewport.rs` 142 ✅；`perf_long_session.rs` 174 ✅。
  - `render.rs` 807→**797** ✅（降低）。
  - **注意（待后续拆分）**：`key_handler.rs` 884→891（HEAD 已超限，本次 +7）；
    `composer.rs` 785→**812**（+27）；`app.rs` 798→**802**（+4）。本次提交先固化已验证改动，
    超限项留待独立拆分迭代（rules/文件行数限制与模块拆分）。

## Impact Surface
- 用户可感知：长会话下 TUI 动画/滚动显著更流畅；Ctrl+C 可取消运行中 turn；超长 transcript
  滚动不再 u16 溢出。
- 不影响：`Store`/`ChatStream` 边界、CLI/Web session 协议、LLM 后端。
- 向后兼容：`EventSink::push` 返回类型新增 `Err(TrySendError::Full(_))` 可能——仅影响
  内部 session 运行时调用方（resume/subagent），已同步适配。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [agents/session](../../agents/session/index.md)
- 既有 [render/vim changelog](../2026-07-26/tui-vim-engine-model-confirm-dialog.md)
