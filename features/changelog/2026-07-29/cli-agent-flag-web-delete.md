Commit: (working-tree, pre-initial-commit)

# feat(cli/web): --agent 全局覆盖 flag + 共享 display 模块 + client --image + DELETE session

## 背景

1. CLI 缺少 `--agent` 全局 flag（已有 `--model`），切换 act/plan/explore/build 必须依赖配置文件；client 模式的 `--agent` 此前是子命令局部字段，无法跨 `run` / 裸 prompt / `client` 复用。
2. `run.rs` 与 `client.rs` 各自维护一份事件渲染逻辑，输出格式容易漂移。
3. client 模式不支持 `--image`（local `run` 已支持），远程提交无法附图。
4. Web API 缺少 session 删除端点，无法清理会话。

## 变更

### CLI `--agent` 全局 flag
- **`crates/cli/src/lib.rs`**：`--agent`（`global = true`）加到 `Cli`；client 子命令的局部 `agent` 字段删除，改用全局 `cli.agent`。
- **`crates/cli/src/run.rs`**：新增 `apply_agent_override`（写 `config.agent.default`，新会话生效）+ `reapply_resume_agent`（显式 flag 覆盖 resume 恢复的 agent 并回持久化）；`run_headless` 接入。
- **`src/main.rs`**：`Client` 分支传 `cli.agent` / `cli.image`（替代原局部字段）。

### 共享 display 模块
- **`crates/cli/src/display.rs`**（新增）：`print_event` + `truncate` / `summarize_input` / `indent_first` 从 `run.rs` 提取；`run.rs` 与 `client.rs` 共用，消除重复。

### client `--image`
- **`crates/cli/src/client.rs`**：`client_run` 增加 `images` 参数，复用 `run::load_image_data_uris`。
- **`crates/client/src/remote.rs`**：`post_prompt` 增加 `images: &[String]`，序列化进 JSON body。

### Web DELETE session
- **`crates/web/src/api.rs`**：`delete_session` 区分 404（不存在）与 200（已删除），幂等。
- **`crates/web/src/lib.rs`**：`/api/sessions/:id` 路由增加 `.delete(api::delete_session)`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| run 接受全局 --agent | `run_subcommand_accepts_global_agent_flag` | cli_parse.rs |
| 裸 prompt 接受全局 --agent | `bare_prompt_accepts_global_agent_flag` | cli_parse.rs |
| client 复用全局 --agent/--model | `client_subcommand_parses_agent_model_interrupt` | cli_parse.rs |
| apply_agent_override | `apply_agent_override_sets_default` | run.rs |
| reapply_resume_agent | `reapply_resume_agent_overrides_stored_agent` | run.rs |
| DELETE session 幂等 | `delete_session_removes_session_and_is_idempotent` | web_contract.rs |

- 全量回归：`cargo test --workspace` → 1300 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：display.rs 148 ≤ 400

## Impact Surface
- CLI：`--agent` 全局生效（run / 裸 prompt / client）；client 模式支持 `--image`。
- Web：`DELETE /api/sessions/:id` 可删除会话（404/200 区分）。
- 不影响：TUI / session / store 内部逻辑。

## Related Docs
- [agents/cli](../../agents/cli/index.md)
- [agents/web](../../agents/web/index.md)
