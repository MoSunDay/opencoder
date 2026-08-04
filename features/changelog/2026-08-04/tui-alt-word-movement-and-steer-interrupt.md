Commit: (working-tree, pre-initial-commit)

# feat(tui): Alt+F/Alt+B 单词移动 + 键盘 Enter steer 立即中断

## 背景

两个独立改进：

1. **单词移动**：composer 光标编辑此前有行首/行尾（Ctrl+A/E）、删词
   （Ctrl+W），但缺 readline 风格按词跳转。长行编辑只能逐字符移动光标，
   效率低。
2. **steer 中断缺失**：键盘 Enter 提交 steer 时，`KeyAction::Steer` 分支
   只 `admit_input` 入库并 push steer_items，但**没有调用中断逻辑**。
   结果 steer 入库后运行中的 turn 不会立即被中断，要等下一个 turn 边界
   才吸收——与 `>` 按钮（SteerSubmit）行为不一致。

## 变更

### 单词移动 — `composer.rs` + `key_handler.rs` + `keybind.rs`

- 新增 `classify_word(char) -> WordKind`（Word=字母数字/下划线、
  Punct=其它非空白、Space=空白）与两个纯函数：
  - `forward_word(input, cursor) -> usize`（Alt+F）：跳过前导空白，消费与
    光标处同类字符，落在词尾之后。
  - `backward_word(input, cursor) -> usize`（Alt+B）：回退一位，跳过尾随
    空白，消费同类字符，落在词首。
  - 无副作用、纯 char-index 运算，覆盖 Unicode（CJK）、标点、换行边界。
- `key_handler.rs`：Alt+F/Alt+B（大小写不敏感）调用上述函数，返回
  `KeyAction::None`（不触发 turn）。放在 Alt+回车 之后、CONTROL 块之前。
- `keybind.rs` 帮助文本新增 `Alt+F / Alt+B` 行。

### steer 中断修复 — `app.rs` + `steer_fire.rs`（新模块）

- 新增 `steer_fire` 模块：提取 `fire_steer_interrupt()`——把原 `app.rs`
  中 SteerSubmit 路径内联的 `steer_dispatch::resolve` + `fire_turn_cancel`
  抽成共享函数，供键盘与鼠标两条路径复用，避免重复与漂移。
- `KeyAction::Steer`（键盘 Enter）分支：提交后**立即调用**
  `steer_fire::fire_steer_interrupt`，与 `SteerSubmit` 对齐——steer 入库
  后马上中断当前 turn（G1 不变式：用 `fire_turn_cancel` 而非硬 abort）。
- `MouseOutcome::SteerSubmit`（`>` 按钮）分支：改用 `fire_steer_interrupt`，
  消除重复代码。
- 事件循环 `tokio::select!` 增加 `biased;`：三臂按声明顺序优先 poll
  （输入臂在前），保证键盘/鼠标 steer 中断不被分支随机化顺序延迟。

## 测试清单

### 单元测试 — 单词移动（`composer_word_tests.rs`，21 例）

forward/backward 正例 + 边界：空串、纯空白、标点串、CJK、换行边界、
行首/行尾、混合标点-单词过渡。

### 单元测试 — steer 中断布线（`steer_fire.rs`，3 例）

- `running_parent_with_steer_fires_turn_cancel` — 运行态 + pending steer →
  返回 `SteerParent` 且 `turn_cancel` 被 fire（核心布线回归）
- `idle_parent_resolves_start_turn_without_firing` — 空闲 → `StartTurn`，不 fire
- `running_parent_with_nothing_pending_is_noop` — 无事可做 → `Noop`，不 fire

### 集成测试 — 键位分发（`app_tests/key_tests.rs`，2 例）

- Alt+F 经 `handle_key` 返回 `KeyAction::None` 且光标前进到词尾
- Alt+B 经 `handle_key` 返回 `KeyAction::None` 且光标后退到词首

### 全量验证

- `cargo test -p opencoder-tui --lib` — **870 passed; 0 failed; 0 ignored**
- `cargo test -p opencoder-session --tests` — **18 passed; 0 failed**
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` — clean
