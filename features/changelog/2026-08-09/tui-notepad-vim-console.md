# feat(tui): notepad 底部面板从单行终端升级为 vim 模态控制台

## 背景

notepad 底部原为单行伪终端面板（`terminal.rs`：`sh -c` 执行命令、滚动历史输出）。
该面板缺乏多行编辑能力，无法在 notepad 内直接撰写长 prompt 或多行 bash 脚本。
本次将其替换为 vim 模态控制台：上方 echo 日志（只读滚动缓冲）+ 下方 Vim composer
（Normal/Insert/Command 三模式），支持 `!`-前缀 bash 执行和 Enter 提交 prompt。

## 变更

### 新增 `notepad/console/` 模块（4 文件）

| 文件 | 行数 | 职责 |
|------|------|------|
| `mod.rs` | 119 | `ConsoleState` 结构体 + echo/vim/bash/running 状态管理 |
| `state.rs` | 180 | `EchoLog` 滚动缓冲（cap 500 行）+ `ConsoleLine`/`ConsoleLineKind` |
| `submit.rs` | 112 | `SubmitKind` 解析（`!`→bash / else→prompt）+ `spawn_bash`（oneshot 包装 `terminal::run_command`） |
| `render.rs` | 217 | 上 echo + 下 composer 分区渲染 + 光标定位 |

### `NotepadOutcome` 新增两 variant

```rust
pub enum NotepadOutcome {
    Exit,
    Consumed,
    SubmitPrompt(String),  // 新：提交文本给 agent session
    RunBash(String),       // 新：后台执行 bash 命令
}
```

`app_notepad.rs`（新，143 行）处理 outcome：
- `SubmitPrompt` → `start_turn` + `console.set_running(true)`
- `RunBash` → `spawn_bash` + oneshot receiver 轮询

### Focus 与键位

- `Focus` 枚举：`{ Tree, Editor, Console }`（底部面板为 `Console`）
- `Tab` 焦点循环：Tree → Editor → Console → Tree
- `toggle_console` keymap 动作（Ctrl+Shift+T）：隐藏/显示底部控制台
- `NotepadView` 新增 `console_hidden: bool` 字段
- `terminal.rs` 保留为 `sh -c` 命令执行辅助（不再作为交互面板）

### `app.rs` 拆分（863 → 799 行）

为满足 ≤800 行迭代上限，将 notepad 集成逻辑提取到 `app_notepad.rs`：
- `key()` — 键盘分发（toggle + dispatch + outcome）
- `paste()` — 粘贴路由到 console vim
- `poll_bash()` — 后台 bash 命令轮询
- `try_render()` — 条件渲染 notepad 或正常 frame

同时提取 shutdown 清理到 `app_bootstrap::finish()`，`set_running` 同步移入
`app_loop::fold_ui_events`。

## 测试覆盖

| 模块 | 测试数 | 覆盖点 |
|------|--------|--------|
| `console/state.rs` | 6 | echo 日志增删/截断/滚动边界 |
| `console/render.rs` | 6 | Normal/Insert/Command 模式渲染、极小区域无 panic |
| `console/submit.rs` | 9 | `!`-前缀解析（含空白/纯`!`边界）、bash 输出捕获 |
| `console/mod.rs` | 3 | 初始模式、reset、bash 完成 |
| `notepad/keys.rs` | 9 | console Esc 退出、Tab 循环、Enter 提交/`!`bash/空提交 noop |
| `notepad/mod.rs` | 5 | SubmitPrompt/RunBash outcome、console_hidden toggle |

- 全量回归：`cargo test --workspace` → **2214 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- 行数：app.rs 799 ≤ 800；app_notepad.rs 143 ≤ 400；console/*.rs ≤ 217 ≤ 400

## Impact Surface

- **用户可见**：notepad 底部面板变为 vim 模态控制台，支持多行编辑和直接提交 prompt
- **不改边界**：不触及 session runner / store / chat 数据形状；prompt 提交复用现有 `UiCmd::Prompt`
- **兼容性**：`terminal.rs` 保留为命令执行辅助，`/notepad` 触发方式不变
