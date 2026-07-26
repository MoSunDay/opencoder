Commit: (working-tree, pre-initial-commit)

# fix(tui): `--model` 显式值覆盖 resume 会话的存储模型并回写持久化

## 背景

已有 changelog `2026-07-25/model-switch-persist-resume.md` 修复了 TUI `/model`
菜单切换的持久化（worker.rs `ReloadConfig` 分支）。但 TUI 启动路径仍有缺口：

`opencode tui -s <id> --model <m>`（或直接 `--model`）时，`run()` 先 resume 出
会话（带其存储的旧模型），随后却未把命令行显式 `--model` 应用上去——用户在
CLI 上指定的模型被 resume 出来的旧模型静默吞掉，且因未回写存储，下次 resume
依然回退。

本轮补齐 TUI 与 headless `run` 路径的 parity。

## 变更

### `crates/tui/src/app_helpers.rs`（两个新 helper）

- `reapply_session_model(&mut session, &Option<String>) -> Option<String>`：
  取 `opts.model`；若与当前 `session.config.model` 相同（或为 `None`）返回
  `None`；否则写入 `session.config.model` 与 `session.model`（
  `config.model_id()` 派生），并返回 `Some(model)` 触发回写。
- `persist_session_model(store, id, model)`：best-effort 经
  `store.update_session(SessionPatch { model, updated_at, .. })` 写回会话行，
  使后续 resume 尊重新选择。

### `crates/tui/src/app.rs`（`run()` 接线）

- 会话创建/resume 之后（`app.rs:101-106`）：若 `reapply_session_model` 返回
  `Some(m)`，即 `await persist_session_model(...)`。

### `crates/tui/src/app_helpers_tests.rs`

- 新增 `reapply_session_model_overrides_resumed_model`：模拟一个存储模型为
  `gpt-4o-mini` 的 resume 会话，传入 `--model anthropic/claude-3`，断言返回
  `Some("anthropic/claude-3")`、`session.model == "claude-3"`、
  `provider_id() == "anthropic"`。

## 回归结果（rules/02-regression-gate）

- `cargo test --workspace` → **1176 passed / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
