Commit: (working-tree, pre-initial-commit)

# fix(tui,cli): idle-resize 轮询安全网 + tracing 输出永不落 tty + supervisor 恢复顺序

## 背景
TUI 渲染/健壮性存在三处独立缺口（继 `tui-render-perf-bounded-events-robustness.md` 之后）：

1. **idle-resize 窗口**：当 crossterm 丢失一次 `Resize` 事件（tmux 分屏/detach-reattach、
   快速拖拽窗口）而界面恰好处于 idle 时，画面会滞留在旧尺寸，直到下一次用户输入才刷新。
   此前事件循环无逐帧尺寸校验，丢失事件即等于丢失重绘。
2. **tracing 可能落入 stdout/stderr**：`init_logging` 经临时 if/else 选 writer，理论上可
   回退到 stdout/stderr，向 alternate-screen 灌入原始转义序列、污染界面。
3. **supervisor 退出顺序颠倒**：input-collector watchdog 退出路径上 `writeln!(stderr,…)`
   先于 `TerminalGuard::restore()` 执行，诊断信息在退出 alt-screen 前即落到 stderr，
   变成覆盖在界面上的转义乱码。

## 变更

### A. idle-resize 轮询安全网（`crates/tui/src/app.rs`）
- **`crates/tui/src/app.rs:47`**：抽出纯函数 `size_changed(prev, cur) -> bool`——无先验
  读数（首帧）或任一维度变化即返回 `true`，使检测逻辑可脱离真实终端做单测。
- **`crates/tui/src/app.rs:284`**：事件循环持有 `last_size: Option<(u16,u16)>`，初始取
  `terminal.size()` 读数。
- **`crates/tui/src/app.rs:789`**：`frame_ticker` 分支每帧以单次 ioctl
  （`terminal.size()`）轮询内核真实尺寸；若 `size_changed` 命中，则强制
  `terminal.autoresize()` + 置 `dirty=true`，闭合 idle-resize 窗口。错误经 `.ok()` 安全跳过。

### B. tracing 输出永不落 tty（`crates/cli/src/lib.rs`）
- **`crates/tui/../cli/src/lib.rs:220`**：新增 `enum LogDest { PrimaryFile, TempFallback,
  Discard }`——该类型**按构造无法表示** stdout/stderr，subscriber 物理上无法污染终端。
- **`crates/cli/src/lib.rs:228`**：纯函数 `log_dest(file_ok, temp_ok) -> LogDest` 决定最佳
  非 tty 目的地（主文件优先 → 临时文件回退 → 丢弃）。
- **`crates/cli/src/lib.rs:197`**：`init_logging` 改为 `let dest = log_dest(...)` 后
  `match` 派发 writer（`:198`），各分支以 `.expect()` 锚定纯函数的不变式。
  **签名未变**，`src/main.rs` 调用兼容。

### C. supervisor 恢复顺序（`crates/tui/src/supervisor.rs`）
- **`crates/tui/src/supervisor.rs:157`**：`trip_reason` 退出分支中
  `TerminalGuard::restore()` 移至 `writeln!(stderr,…)` **之前**（`:158`）。先退出
  alt-screen/raw 模式，stderr 诊断方可安全显示（或 tty 已失联时无害丢弃）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| idle-resize 任一维度变化被检测 | `size_changed_detects_dimension_change` | `crates/tui/src/app_tests.rs` |
| 维度未变返回 false | `size_changed_false_when_unchanged` | `crates/tui/src/app_tests.rs` |
| 首帧无先验读数返回 true（边界） | `size_changed_true_when_no_prior_reading` | `crates/tui/src/app_tests.rs` |
| 主日志文件优先（正常路径） | `log_dest_prefers_primary_file` | `crates/cli/src/lib.rs` |
| 无主文件回退临时文件 | `log_dest_falls_back_to_temp_when_no_primary` | `crates/cli/src/lib.rs` |
| 主/临时皆无才丢弃 | `log_dest_discards_only_when_both_unavailable` | `crates/cli/src/lib.rs` |

- 全量回归：`cargo test --workspace` → **1204 passed / 0 failed**（exit 0，本会话复跑取证）。
  - 注：`viewport_build_and_slice_5k_blocks`（`crates/tui/tests/perf_long_session.rs`）
    本次通过；该测试为**既有 flaky 时序断言**（`build_ms < 1000ms`），在 workspace 并行
    负载下偶发失败、隔离单跑与 Release 均稳定通过（3/3）。其测试的 `render_viewport.rs`
    不在本轮 diff 内，属既有问题，建议在独立任务中硬化（放宽阈值或限定 Release）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（exit 0）。
- 构建：`cargo build --workspace` → 干净零 error（exit 0）。
- 防修绿扫描：无 `#[ignore]`、无删除 `#[test]`、无弱化断言、无调试输出、无密钥。
- 行数（新文件 ≤ 400，迭代中文件 ≤ 800）：
  - 本轮**无新文件**。
  - `crates/cli/src/lib.rs` 257 ✅；`crates/tui/src/supervisor.rs` 225 ✅。
  - **注意（既有超限，待后续拆分）**：`app.rs` 802→**825**（+23，HEAD 前 802 已超 800）；
    `app_tests.rs` 1300→**1319**（+19，HEAD 前 1300 已超 800）。二者均为既有超限，本轮新增
    为修复所必需的最小量；超限项留待独立拆分迭代（rules/文件行数限制与模块拆分）。

## Impact Surface
- 用户可感知：tmux 分屏/快速拖拽窗口后 TUI 不再滞留于旧尺寸（idle 期间每帧轮询内核尺寸）；
  日志永不污染 alternate-screen；watchdog 退出提示正常显示而非转义乱码。
- 不影响：`Store`/`ChatStream` 边界、CLI/Web session 协议、LLM 后端。
- 向后兼容：纯内部 robustness 修复，无 trait/shape/config 变更；`init_logging` 签名未变。

## 风险与对齐
- **纯函数式**：`size_changed` 与 `log_dest` 均为纯函数（无副作用）；`LogDest` 为不可变
  枚举，stdout/stderr 在类型层面不可表示。状态经参数/返回值传递，非对象内部状态。
- **默认行为不变**：3FPS 渲染节流**未改动**；日志仍默认写主日志文件，仅在其不可用时
  回退临时文件、再不可用才丢弃——绝不会落 tty。
- **范围外（提交排除）**：14 个既有前序会话脏改动（llm/session/tui 的 image/compaction/
  vim/app_helpers 等）未纳入本轮提交，commit 仅含 4 个在范围内文件 + 本 changelog。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [agents/cli](../../agents/cli/index.md)
- 既有 [TUI 渲染性能与有界事件健壮性](../2026-07-26/tui-render-perf-bounded-events-robustness.md)
