# ChatView 接管回合开始 / steer 消费呈现状态（可测试化重构）

## 背景

两个已报告行为此前埋在 `run_app` 的私有事件循环里，无单测缝隙：

1. **回合开始清状态**：每个 turn 开始时需清掉上一 turn 残留的瞬时 `status`（如 `[interrupted]` 中断态）。此清理由 `run_app` 内散落的 5 处 `chat.status.clear()` 完成，缺一处即泄漏到状态栏——纯靠肉眼，不可测。
2. **steer 消费 marker**：steer 在 turn 边界被提升时，应向 transcript 推入一条青色 `steer: {prompt}` marker，让用户看见它**何时**真正介入（区别于提交时固定显示在侧栏的 echo）。此前由 `run_app` 内一段无法单测的 `if let SessionEvent::SteerConsumed` 块实现，且该块还要同时操作一个**本地** `steer_items` 镜像，与 `SessionUiState.steer_items` 字段构成重复事实来源。

目标：让 `ChatView` 成为呈现状态的唯一拥有者，`run_app` 退化为轻量调度器；两个行为都获得单元测试；消除重复 `steer_items` 来源。

## 变更

### `crates/tui/src/chat.rs`

- 新增字段 `pub steer_items: Vec<(i64, String)>`（在 `status` 之后）。`ChatView` derive `Default`，所有既有字面量均用 `..Default::default()`，加字段安全。
- 新增 `pub fn begin_turn(&mut self) { self.status.clear(); }`——回合开始不变式的唯一拥有者（在 `push_marker` 之前）。
- `apply` 中 `SessionEvent::SteerConsumed { seq }` 从空操作改为真正处理：先 clone 文本释放共享借用，按 seq 查找 prompt，推入青色加粗 `steer: {prompt}` marker + 空白间隔行，再 `retain` 移除该 seq。未知 seq 为 no-op。

### `crates/tui/src/app.rs`

- 还原导入 `{Color, Modifier, Style}` → `{Color, Style}`（删 `Modifier`，已无引用）。
- 删除本地 `let mut steer_items` 变量；全部 8 处使用点改为 `chat.steer_items`（render 调用、snapshot 调用、`/task` 恢复 `chat.steer_items = st.chat.steer_items.clone()`、从 store 重建、`KeyAction::Steer` 推送、`Done` 清除、`clear_pending_inputs` 子借用）。
- 全部 5 处 `chat.status.clear()` → `chat.begin_turn()`；并为 `MouseOutcome::SteerSubmit` 空闲分支补上 `chat.begin_turn()`，使不变式「每个回合开始都调用 `begin_turn`」无特例（共 6 处）。
- `handle_mouse` 调用删除 `&mut steer_items` 参数（已有 `&mut chat`）。
- 删除整段 `if let SessionEvent::SteerConsumed` 块——逻辑已迁入 `apply`，借用冲突随之消失。

### `crates/tui/src/app_helpers.rs`

- `handle_mouse` 签名删除 `steer_items: &mut Vec<(i64, String)>` 参数；函数体 `retain` → `chat.steer_items.retain(...)`。`clear_pending_inputs` 签名不变。
- 12 处测试调用点同步删除参数；空声明删除；`submit_btn_returns_steer_submit` 的初值/断言改为 `chat.steer_items`。

### `crates/tui/src/session_ui.rs`

- 删除 `SessionUiState.steer_items` 字段、`new()` 初始化、`snapshot()` 参数与函数体行——`chat.clone()` 现自带 steer_items（单一事实来源下沉到所有权层）。3 处测试同步修正（`snap.steer_items` → `snap.chat.steer_items`，删多余参数，snapshot 前把 steers 写入 `chat.steer_items`）。

### `crates/tui/src/render_tests.rs`

- 2 处 `handle_mouse` 调用点同步删参数 + 删空声明（`render.rs` 渲染函数签名不变，调用方传 `&chat.steer_items`）。

净改动：`app.rs` 1046 → 1027 行（-19）；`chat.rs` 753 → 785（+`begin_turn` + `steer_items` 字段 + 处理器）；重复 `steer_items` 事实来源消除。

## 测试覆盖

新增 4 个 `ChatView` 单元测试（`crates/tui/src/chat_tests.rs`）：

| 测试 | 验证不变式 |
|------|-----------|
| `begin_turn_clears_status` | `Status("interrupted")` → `begin_turn` 后 `status` 为空 |
| `begin_turn_preserves_transcript` | `begin_turn` 不影响 transcript blocks |
| `steer_consumed_pushes_marker_and_drops_entry` | `SteerConsumed{7}` 推入 `steer: use python` marker 且 `steer_items` 清空 |
| `steer_consumed_unknown_seq_is_noop` | 未知 seq 不推 marker、条目保留 |

底层 steer 提升契约仍由既有集成测试覆盖：

| 保留逻辑 | 覆盖测试 | 文件 |
|----------|----------|------|
| `claim_steers` / `SteerConsumed { seq }` 提升语义 | `steer_consumed_carries_pk_seq_not_admitted_seq` 等 7 项 | `crates/session/tests/steer_followup.rs` |
| `MouseOutcome::SteerSubmit` 点击检测 | `submit_btn_returns_steer_submit` | `crates/tui/src/app_helpers.rs` |
| snapshot 不含 steer_items 字段后的独立性 | `snapshot_is_independent_of_source` | `crates/tui/src/session_ui.rs` |

> 诚实说明：`run_app` 内 `begin_turn()` 调用点本身属 live 终端事件循环，未被单测直接驱动；其安全性依据是「行为已下沉为 `begin_turn`/`SteerConsumed` 处理器，二者各自有单元测试 + 6 处调用点机械等价替换」，而非对循环本身的端到端测试。

## Gate（当次实跑取证）

| 项 | 结果 |
|----|------|
| `cargo check -p opencoder-tui --tests` | 零错误 |
| `cargo clippy -p opencoder-tui --all-targets -- -D warnings` | 零警告（含已删 `Modifier` 导入） |
| `cargo test -p opencoder-tui` | 304 passed / 0 failed（300 既有 + 4 新增） |
| `cargo test -p opencoder-session --test steer_followup` | 7 passed / 0 failed |
| `cargo test --workspace` | **629 passed / 0 failed / 0 ignored** |

> 计数说明：基线 625 + 本次新增 4 = 629，全部通过。无任何 `#[test]` 被删 / 新增 `#[ignore]` / 弱断言替换。
