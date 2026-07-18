# 全局 agents.md 不计入 ctx token 预算

## 背景

启动 opencoder（以及整个会话期间）时，全局 `~/.opencode/AGENTS.md` 的内容会被 `build_system()` 注入系统消息并随每个 turn 发给模型，其 token 同时被计入两处 ctx token 统计：

1. 压缩预算估算（`compaction.rs::estimated_tokens` / `reported_tokens`）—— 一个大的全局文件会提前吃掉会话对话窗口、过早触发压缩；
2. TUI 上下文计量条（`app_helpers.rs::sys_tokens_for`）—— 刚启动、尚无任何消息时计量条就已被全局文件撑大，对用户造成误导。

全局文件是「常驻基线上下文」，不应占用单会话的对话预算。需求：**全局 agents.md 仍照常发送给模型，但其 token 不计入 ctx 预算/计量。** 本地（git-root / working-dir）的 agents.md 不受影响，继续计入。

## 变更

### 新增 `global_instructions_text`（`crates/session/src/prompt.rs`）

`pub fn global_instructions_text(working_dir) -> Option<String>`：只读取全局 `~/.opencode/AGENTS.md` 的内容（trimmed），用于在各计费点扣除。返回 `None` 的情形：文件缺失/不可读/为空，或全局目录与 working_dir 同一目录（此时该内容已作为本地指令计入，不重复扣除）。复用现有 `find_agents_md`，未新增文件 I/O 逻辑。

### 三处计费点扣除全局 token

- `compaction.rs::estimated_tokens`（估算信号，round-1 即生效）：在系统消息 token 上 `saturating_sub` 全局 token。
- `compaction.rs::reported_tokens`（权威信号，模型上报的真实 input_tokens）：同样扣除全局 token，保证两个信号一致——否则大全局文件会经此路径重新计入预算、过早触发压缩，使估算路径的扣除形同虚设。
- `app_helpers.rs::sys_tokens_for`（TUI 计量条）：扣除全局 token，启动即生效。

全局内容**仍照常**随系统消息发送给模型（`runner.rs::run_one_llm_call` 未改动），仅预算/计量口径排除。

### 溢出安全性

默认配置 `context_limit=128_000`、预算 `min(threshold=80_000, usable=108_000)=80_000`，留有 **48k token** 余量。即便扣除全局后压缩触发稍晚，任何现实大小的全局 agents.md（需 >48k token ≈ 192k 字符才会逼近硬上限）都在安全余量内；且 `reported_tokens` 仍以真实上报值为基线扣除，`reserved` 保留余量。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 全局 agents.md 内容被正确提取 | `global_instructions_returns_global_agents_md_content` | `session/tests/prompt.rs` |
| 无全局文件时返回 None | `global_instructions_none_when_no_global_file` | `session/tests/prompt.rs` |
| 全局文件为空时返回 None | `global_instructions_none_when_global_file_empty` | `session/tests/prompt.rs` |
| git-root/working-dir 文件不计入全局 | `global_instructions_ignores_git_root_and_working_dir_files` | `session/tests/prompt.rs` |
| 全局 agents.md 不影响压缩预算（差分验证） | `global_agents_md_excluded_from_compaction_budget` | `session/tests/compaction_and_model.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 574 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |
