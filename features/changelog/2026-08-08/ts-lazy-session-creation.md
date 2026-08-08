# fix(session,cli,tui): ts 启动不再预写空 session 行

## 背景

`enable_tmux_session` 启用时，每次执行 `opencode`（bare → tmux 包装）会在用户提交任何
input 之前就向 store 写入一条空 session 行（`model: None, agent: None, title: None`）。
根因链：

```
opencode (bare) → main.rs:116 maybe_wrap_tui_in_tmux
  → ts/actions.rs ts_start
    → ensure_session(&workdir, &id)
      → store.create_session(SessionMeta { model: None, ... })   ← 零消息空行
```

而普通 TUI 路径（无 tmux）是惰性的——首条 `record()` → `persist()` 才写行。

## 变更

### `crates/session/src/lib.rs`

- `SessionState` 新增私有字段 `ts_origin: bool`（`new()` 默认 `false`）。
- 新增 consuming builder `.ts_origin()` 设置为 `true`。
- `persist()` 首次创建行时：`ts_origin == true` 则写 `model: None` / `agent: None`
  （ts-ownership 持久标记），否则照旧写 `Some(model)` / `Some(agent)`。内存中
  `self.model` / `self.agent` 不受影响（始终来自 config）。

### `crates/cli/src/ts/actions.rs`

- `ts_start` 中 `ensure_session(&workdir, &id)` → `record_workdir(&workdir)`
  （仅写 workdir 标记文件，不写 session 行）。
- 删除 `ensure_session` 函数及仅供其调用的 `open_store_for` 辅助函数。

### `crates/tui/src/app_bootstrap.rs`

- `--session <id>` 给定但 store 中不存在且非 subagent task 时，不再走
  `resume_and_replay`（会报 `"session not found"`），改为创建新鲜 `SessionState`
  并链 `.ts_origin()`——行在首次 `record()` 时惰性落盘，`model: None` 标记保留。

### `crates/session/src/resume.rs`

- `resume()` 构造 `SessionState` 时补 `ts_origin: false`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ts_origin session：首次 record 前 store 无行 | `ts_origin_no_row_before_first_record` | `session/tests/ts_origin_persist.rs` |
| ts_origin session：record 后 model/agent 为 None | `ts_origin_persists_null_model_and_agent` | 同上 |
| 普通 session：record 后 model/agent 已落库 | `normal_session_persists_model_and_agent` | 同上 |
| resume 后 ts_origin session 保持 model: None | `ts_origin_resume_keeps_null_model` | 同上 |

## 回归

- `cargo test --workspace --no-fail-fast` → **2125 passed / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- `cargo build --workspace` → 编译干净

## 影响面

- `ts -l` 不再列出"已启动但未提交 prompt"的空 session——`is_registered` 本来就
  过滤空 session（`has_content` 要求 preview 或 title 非空），行为符合预期。
- `ts -r <id>` 恢复已提交 prompt 的 ts session 仍然正常（行存在，`model: None`）。
- 旧的种子残留行（修复前已创建的）属预存数据，用户可用 `ts -c` 清理。
