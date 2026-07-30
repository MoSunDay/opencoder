Commit: (working-tree, pre-initial-commit)

# feat(tui): /ps /stop display-only, context-free background-bash commands

## 动机

后台 bash（bash 超时后 handoff 到 detached supervisor 的进程）此前只能通过
重启程序或自然退出清理，用户在 TUI 里既看不到当前有哪些后台进程，也无法主动
结束它们。需要一个**完全不进入模型上下文**的观测/控制手段——命令本身和它的结果
都只对用户可见，绝不写入 `session.messages`。

## 设计

两个**纯展示、无上下文**的斜杠命令，紫色（`theme::LOCAL = Magenta`，与既有
`[model]` 标记同色系，表示「不进上下文的本地信息」）渲染：

- `/ps` — 列出所有已注册的后台 bash 进程（pid + 输出文件路径）。
- `/stop` — 强制结束所有后台 bash 进程组，返回固定英文信息
  `[stop] Process has been forcibly terminated.`（空注册表时仍返回该统一信息）。

**为何不进上下文**：结果通过 `ChatView::push_marker_lines` 写入 TUI 本地
`ChatBlock::Marker`，与 `user:` echo、`[model]`、`[context compacted]` 等本地
标记走同一条「永不 `record()`、永不 `start_turn`」的通道。结构上等价于已有的
不进上下文标记。

**两条入口**：
1. 弹窗（主路径）——`/` 在空输入时打开 picker，选中 `/ps`/`/stop` 回车 →
   `CommandOutcome::Dispatch(Ps|Stop)` → `dispatch_command` 调
   `local_cmd::run`。Tab 对非控制命令保持弹窗打开（与现有非控制命令一致）。
2. 空闲态自由文本/粘贴兜底——`app.rs` 的 `KeyAction::Submit` 在计算 `clean`
   后，**作为首个分支**调用 `local_cmd::run`；命中则不 echo、不 start_turn、不
   进上下文，直接跳出。

`/ps`/`/stop` 在任意状态（空闲 + turn 进行中）均可执行，因为它们从不触发
`start_turn`。

## 改动

### `crates/session/src/tools/bg.rs`
- 新增 `pub struct BgInfo { pid: u32, output_path: PathBuf }`——注册项的**公共**
  快照（不暴露 `Child`/handle）。
- 新增 `pub fn list() -> Vec<BgInfo>`——快照私有注册表。
- 新增 `pub fn kill_all() -> usize`——`drain()` + `kill(-pgid, SIGKILL)`（`#[cfg(unix)]`）
  + `remove_file` 输出文件，返回击杀数。
- 重构 `cleanup_all()` 为 `let _ = kill_all();`（去重，行为不变）。

### `crates/tui/src/theme.rs`
- 新增 `pub const LOCAL: Color = Color::Magenta;`（doc：本地/不进上下文信息）。
- 新增 `pub fn local_style() -> Style`。

### `crates/tui/src/local_cmd.rs`（新文件，134 行）
- `pub(crate) fn run(text, chat) -> bool`——识别 `/ps`/`/stop`，推送紫色标记；
  未识别返回 `false`。
- `fn format_ps(procs) -> Vec<Line>`（纯函数）：空 → `[ps] no background processes`；
  非空 → 标题 `[ps] background bash (N):` + 每行 `pid  /tmp/opencoder_bg_<pid>.output`。
- `fn is_local(text) -> bool`。

### `crates/tui/src/chat.rs`
- 新增 `pub fn push_marker_lines(lines: Vec<Line>)`——以单个多行 `Marker` 块渲染
  `/ps`（`finalize_assistant` + push）。

### `crates/tui/src/command.rs`
- `SlashAction` 新增 `Ps`、`Stop` 变体。
- `COMMANDS` 新增 `/ps`、`/stop`（中文描述，标注「不计入模型上下文」）。
- `parse`/`dispatch` 新增对应分支；`control_cmd_string` 经 `_ => None` 自动返回
  `None`（不可 Tab 排队，回车立即执行）。

### `crates/tui/src/app_loop.rs`
- `dispatch_command` 新增 `Dispatch(Ps)`/`Dispatch(Stop)` 两条 arm，各调一次
  `local_cmd::run`，落到底部 `LoopFlow::Proceed`（依赖事件到达时已置
  `dirty = true`，帧 tick 自动重绘紫色标记）。

### `crates/tui/src/app.rs`
- `KeyAction::Submit` 计算 `clean` 后，以 `if crate::local_cmd::run(&clean, &mut chat)`
  作为首个分支实现空闲自由文本/粘贴兜底。

## 行数

bg.rs 296 · local_cmd.rs 134（新）· theme.rs 230 · command.rs 571 · chat.rs 786 ·
app_loop.rs 792 · app.rs 800（均 ≤ 800）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `kill_all` 真实进程往返（setsid sleep，pid+路径，击杀后清空） | `kill_all_terminates_registered_group_and_drains_registry` | `crates/session/tests/bg_kill_all.rs` |
| `/ps` 格式化：空 | `format_ps_empty` | `crates/tui/src/local_cmd.rs` |
| `/ps` 格式化：1 条 | `format_ps_one` | `crates/tui/src/local_cmd.rs` |
| `/ps` 格式化：多条 | `format_ps_many` | `crates/tui/src/local_cmd.rs` |
| 命令识别：命中 | `is_local_matches` | `crates/tui/src/local_cmd.rs` |
| 命令识别：非命中 | `is_local_non_matches` | `crates/tui/src/local_cmd.rs` |
| `/stop` 固定文案 | `stop_message_text` | `crates/tui/src/local_cmd.rs` |
| `parse` 解析 /ps /stop | `parse_local_commands` | `crates/tui/src/command.rs` |
| 弹窗 Enter → Dispatch(Ps) | `enter_on_ps_dispatches` | `crates/tui/src/command.rs` |
| 弹窗 Enter → Dispatch(Stop) | `enter_on_stop_dispatches` | `crates/tui/src/command.rs` |
| Tab 对 Ps 为 Idle（弹窗保持） | `tab_on_local_command_is_idle` | `crates/tui/src/command.rs` |
| `control_cmd_string(Ps/Stop)` => None | `control_cmd_string_maps_correctly` | `crates/tui/src/command.rs` |

- 全量回归：`cargo test --workspace` → **1396 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误

## 手动验证（待人工）

1. 触发一次 bash 超时 handoff（短 timeout + 长命令）。
2. `/ps` → 紫色列表含 pid + `/tmp/opencoder_bg_<pid>.output`。
3. `/stop` → 紫色 `[stop] Process has been forcibly terminated.`。
4. 再 `/ps` → 紫色 `no background processes`。
5. `opencode session show --json` 确认 `session.messages` 中**无** `/ps`/`/stop`
   文本及其输出（仅本地标记，不进上下文）。

## 风险与取舍

- **全局注册表并行测试**：`kill_all()` 会 drain 进程级全局注册表并组杀全部已注册
  进程；若放在 lib 单测二进制内，会与并行运行的 bash 注册测试互相干扰（drain 掉
  对方条目 / SIGKILL 对方 `sleep`）。故将其覆盖移至独立集成测试二进制
  `crates/session/tests/bg_kill_all.rs`（每个 `tests/*.rs` 为各自独立二进制、
  独立全局注册表，互不干扰）；lib 内 bg 单测仅触及自身 pid，`list()` 覆盖由
  `register_unregister_roundtrip` / `stop_kills_registered_process` 提供，天然并行安全。
- **`kill(-pgid)` 安全**：测试子进程经 `setsid`，`pgid == pid`，组杀仅影响该
  `sleep`，不波及测试 runner；末尾 `child.wait()` 回收僵尸。
- **`/stop` 统一文案**：空注册表时仍返回统一信息（按要求）；后续如需区分可改。
- `bg::list()`/`kill_all()` 为公共 API，CLI/web 可后续复用，本轮仅接 TUI。
