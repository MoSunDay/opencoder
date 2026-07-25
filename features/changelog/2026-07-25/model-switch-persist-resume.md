Commit: (working-tree, pre-initial-commit)

# fix(tui/session): `/model` 切换持久化到存储，resume 不再回退模型

## 背景

TUI 的 `/model` 菜单（以及 web 的 `POST /sessions/:id/model`）通过
`UiCmd::ReloadConfig` 在 turn 边界热切换模型。修复前，切换只更新了内存中的
`SessionState.model`，而 `sessions.model` 这一持久化列从未被写入。

后果：下一次 `resume()`（`opencode -s <id>` 或 `/task resume`）会从存储读回
**旧的** `sessions.model`，静默地把模型回退到切换前的值——用户看到的 `/model`
切换在重启后「消失」了。

## 变更

### `crates/session/src/runner/event.rs`（新增事件变体）

- 新增 `SessionEvent::ModelSwitch(String)`，携带 `"provider/model_id"` 字符串，
  使展示层与 resume 保持与磁盘配置一致。
- 补齐 4 个 match 分支：`sse_kind()` → `"model_switched"`、`sse_data()` →
  `{ "model": m }`、`from_sse()`（逆解析）、`coarse_kind()` → 复用既有
  `EventKind::ModelSwitched`（此前已映射但未被任何事件使用）。
- 联调单元测试 `from_sse_roundtrips_all_variants` 已覆盖新变体的序列化往返
  （严格相等断言）。

### `crates/tui/src/worker.rs`（`ReloadConfig` 分支持久化 + 发送事件）

- 在 `process_cmd` 的 `ReloadConfig` 分支尾部，当模型已应用时：
  - 通过 `store.update_session(SessionPatch { model: Some(..), updated_at: Some(..), .. })`
    把新模型写回存储（best-effort，`let _`）——这是本 bug 的根因修复。
  - 发送 `SessionEvent::ModelSwitch(model_value)`：先 `persist_event`（SSE/web
    回放可观测），再 `forward_event` 推给 TUI 通道（生命周期事件，永不丢弃）。
- 三条路径（成功 / 客户端构建失败 / endpoint 解析失败）均会执行持久化 + 发送，
  保证「即使降级到旧客户端，配置也已落盘」。

### `crates/tui/src/chat.rs`（渲染消费）

- `ModelSwitch` 分支渲染一个 magenta `[model] <m>` 标记块（`finalize_assistant`
  后推送 `ChatBlock::Marker`）。

### `crates/cli/src/run.rs`（headless 消费）

- `ModelSwitch(to)` 打印一行 `\n[switched to model: {to}]`（magenta）。

## 测试清单（rules/02-regression-gate）

| 命令 | 结果 |
| --- | --- |
| `cargo test -p opencoder-session --lib -- event::` | 3 passed / 0 failed（含 17 类 `from_sse` 往返，覆盖 `ModelSwitch`） |
| `cargo test -p opencoder-session --test compaction_and_model` | 10 passed / 0 failed |
| `cargo test -p opencoder-session --test config_reload` | 4 passed / 0 failed |
| `cargo test -p opencoder-web` | 32 passed / 0 failed（6 lib + 6 auth + 4 client_e2e + 1 replay_fidelity + 9 web_contract + 6 web_drain_contract） |
| `cargo test -p opencoder-tui --test model_switch_persist` | 2 passed / 0 failed（**新增**：worker 持久化 + 事件发送；resume 不回退） |
| `cargo clippy -p opencoder-session` | 零警告 |
| `cargo build --workspace` | Finished dev profile，0 错误 |

新增测试 `crates/tui/tests/model_switch_persist.rs`（2 个）：

1. `reload_config_persists_model_and_emits_model_switch_event` —— 用真实
   `LibsqlStore::open_memory()` 构造带存储的会话，断言 (a) `ReloadConfig` 后
   `get_session().model == "openai/test-model"`（持久化生效），(b) 事件通道收到
   `UiEvent::Session(SessionEvent::ModelSwitch("openai/test-model"))`。
2. `model_switch_survives_resume` —— 端到端证明：`process_cmd(ReloadConfig)`
   落盘后，对同一会话用**旧的**默认 config 调 `resume()`，断言
   `resumed.model == "test-model"`（而非回退到 `gpt-4o-mini`）。这是 bug 修复的
   直接证据。

## 风险与对齐

- **回归风险：低。** 新变体的所有 `SessionEvent` match 分支已覆盖；SSE 往返测试
  通过；`coarse_kind` 复用既有 `EventKind::ModelSwitched`（store/web 已有编解码）。
- **持久化为 best-effort**：`update_session` / `append_event` 结果以 `let _` 忽略，
  与仓库既有「存储失败不阻塞主流程」约定一致（无存储的内存会话不受影响）。
- **纯函数式 / 无 class**：本次仅新增 enum 变体与 match 分支，未引入类或可变内部
  状态，符合仓库「纯函数式编程」规则。
- **范围外**：工作区中其它脏文件（`image_render.rs`、`render.rs`、
  `chat_types.rs`、`plan_edit.rs`、`bg.rs` 等）的编译/clippy 错误与本次修复无关，
  提交时需排除。
