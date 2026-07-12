Commit: (working-tree, pre-initial-commit)

# 修复 skill 在 session 运行中激活时不生效的 bug

## 背景

当 session 正在运行时（`run_loop` 内部），用户提交带 `{$submit}` token 的文本：

1. `apply_skill_tokens` 通过 cmd channel 发送 `UiCmd::SetSkill` — 但 worker 正阻塞在 `run_loop` 内，不会处理该命令直到 `run_loop` 返回
2. prompt 文本通过 `store.admit_input(Delivery::Queue)` 进入 store 队列
3. `run_loop` 在 idle boundary 直接从 store 消费队列项 → 下一轮 `run_one_llm_call` 读取 `session.skill_prompt` — 仍是旧值（None）
4. `run_loop` 返回后，worker 才处理 `SetSkill` — 但此时已没有更多 turn 了

结果：skill 永远不会应用到需要它的那个 turn。Submit/Steer/Queue 三条路径在 session running 时都受影响。

## 变更

### A — `skill_prompt` 改为共享可变状态

#### `crates/session/src/lib.rs`
- `skill_prompt` 类型从 `Option<String>` 改为 `Arc<Mutex<Option<String>>>`
- `new()` 初始化为 `Arc::new(Mutex::new(None))`
- `with_skill()` 改为经 Mutex 写入
- 新增 `pub fn skill_prompt_cloned(&self) -> Option<String>` — 快照当前 skill（克隆内层 String）
- 新增 `pub fn set_skill(&self, body: Option<String>)` — 原地更新 skill（`&self`，不需 `&mut`）

#### `crates/session/src/runner.rs`
- `run_one_llm_call` 中 `session.skill_prompt.as_deref()` → `session.skill_prompt_cloned().as_deref()`，每 turn 读取最新值

#### `crates/session/src/compaction.rs`
- `estimated_tokens` 中同上模式

#### `crates/session/src/resume.rs`
- `skill_prompt: None` → `skill_prompt: Arc::new(Mutex::new(None))`

### B — TUI 直接更新 Arc，绕过 cmd channel

#### `crates/tui/src/app_helpers.rs`
- `apply_skill_tokens` / `resolve_and_warn` 参数从 `cmd_tx: &mpsc::Sender<UiCmd>` 改为 `skill_handle: &Arc<Mutex<Option<String>>>`
- skill 激活从 `cmd_tx.send(UiCmd::SetSkill(Some(body))).await` 改为 `*skill_handle.lock().unwrap() = Some(body)`（直接写入共享 Arc）
- 两函数不再需要 `async`（原 async 仅因 `cmd_tx.send().await`），改为同步函数

#### `crates/tui/src/app.rs`
- `run_app` 中 `session.with_cancel(...)` 后添加 `let mut skill_handle = session.skill_prompt.clone()`
- 3 处 `resolve_and_warn` 调用：`&cmd_tx` → `&skill_handle`
- `KeyAction::SetSkill(Some/None)`：`cmd_tx.send(SetSkill(...))` → `*skill_handle.lock().unwrap() = ...`
- `/task` switch：克隆新 session 的 `skill_prompt` Arc 并重绑定 `skill_handle`；sticky skill 直接写入新 Arc

#### `crates/tui/src/worker.rs`
- `UiCmd::SetSkill` handler：`sess.skill_prompt = body` → `sess.set_skill(body)`（保留 enum variant 供未来 web/CLI 使用）

#### `crates/tui/src/app_tests.rs`
- 3 处 `apply_skill_tokens` 测试调用：简化 `block_in_place + block_on` 为直接同步调用（函数不再是 async）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| skill 在运行中经 steer 路径激活后出现在下一 turn 的系统提示 | `skill_set_mid_run_appears_in_next_turn_system_prompt` | `crates/session/tests/skill_mid_run.rs` |
| skill 在运行中经 queue 路径激活后出现在 follow-up turn 的系统提示 | `skill_set_mid_run_appears_in_queue_followup_turn` | `crates/session/tests/skill_mid_run.rs` |
| `set_skill` / `skill_prompt_cloned` 往返 | `set_skill_and_clone_roundtrip` | `crates/session/tests/skill_mid_run.rs` |
| `with_skill` builder 设置 skill | `with_skill_builder_sets_skill` | `crates/session/tests/skill_mid_run.rs` |
| TUI apply_skill_tokens 解析已知 skill 并写入 skill_handle | `apply_skill_tokens_resolves_and_activates_known_skill` | `crates/tui/src/app_tests.rs` |
| TUI apply_skill_tokens 报告未知 skill | `apply_skill_tokens_reports_unknown_skill` | `crates/tui/src/app_tests.rs` |
| TUI apply_skill_tokens 无 token 时不动 skill | `apply_skill_tokens_no_tokens_leaves_skill_untouched` | `crates/tui/src/app_tests.rs` |
| build_system 追加 skill 段 | `build_system_appends_skill_when_provided` | `crates/session/tests/prompt.rs` |
| build_system 空 skill 不追加段 | `build_system_omits_skill_section_when_empty` | `crates/session/tests/prompt.rs` |
| worker SetSkill handler 调用 set_skill | — | `crates/tui/src/worker.rs`（签名检查，clippy 保证） |

- 全量回归：`cargo test --workspace` → 470 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
