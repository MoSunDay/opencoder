# feat(cli): `ts` 子命令 Store-first 统一会话管理 + `-c` 清理

## 背景

此前 `opencode ts`/`rs` 的语义是 **tmux-first**：裸 `ts` 恒新建一个 tmux
托管 session，`-l` 只列出 tmux 中存活的 session，`-r <id>` 只能重连存活
session。这导致：

- Store 里已持久化但 tmux 进程已退出的 session（stopped）**不可见、不可恢复**。
- 用户每次裸 `ts` 都得到新 session，无法快速回到最近的工作上下文。
- stopped session 堆积在 Store 中无法清理。

本次改为 **Store-first** 统一面：以 Store 为会话的唯一事实来源，tmux 仅作
运行时存活标记。`-l` 列全部（live + stopped），`-r` 可冷启动 stopped
session，裸 `ts` 复用最近 live session，新增 `-c` 清理 stopped session。

## 变更

### `ts` 语义全面重写 — `ts/actions.rs`（+206/−87）

- **`ts_start(cli, force_new)`**：裸 `ts` 不再恒新建。非强制时优先复用——
  若 `--session <id>` 存活则 attach；否则 attach 最近 live session；都没有
  才新建。`--new`（`force_new=true`）跳过复用，恒新建。
- **`ts_list(cli)`**：Store-first 统一面。从 Store `list_sessions` 拉全部
  session，与 tmux `list_managed` 交叉标注存活状态。列：`marker id8
  created-ago workdir task-head`（`*`=attached、`·`=detached、空=stopped）。
  排序：非 stopped 优先 → workdir 路径升序 → 创建时间降序（同路径组内最新
  在前）。
- **`ts_resume(cli, target)`**：存活则 attach；stopped 则从 Store 历史冷启动
  （`spawn_session` 重新拉起 tmux）。Store 中不存在才报错。
- **`ts_cleanup(cli)`**（新）：删除 Store 中存在但 tmux 中已不存活的 session。
- **内部纯函数**：
  - `classify(id, tmux_map) -> TmuxState`（Attached / Detached / Dead 三态）
  - `is_stopped(path_display) -> bool`（`(stopped)` 前缀判定，用于排序与清理）

### 显示辅助函数 — `ts/display.rs`（+47）

- `id8(id) -> String`：截取 id 前 8 字符（紧凑列表列）。
- `ms_to_secs(ms) -> i64`：毫秒 epoch → 秒（供 `format_ts`）。
- `preview_of(preview, title) -> &str`：优先 `/task` preview，空则回退 title。

### CLI flag 扩展 — `lib.rs` + `main.rs` + `cli_parse.rs`

- `Command::Ts` 新增 `clean: bool`（`-c`/`--clean`）。
- `--new` 语义从 "无操作（向后兼容）" 改为 "强制新建，跳过复用"。
- `main.rs`：所有 pattern-match 点（`ts_dispatch` 调用、`runs_inline`）更新
  签名透传 `clean`。
- `main.rs` 新增 `maybe_wrap_tui_in_tmux(cli)`：当 `enable_tmux_session`
  配置为 true 且 tmux 可用且当前不在 tmux 内时，裸 `opencode` 自动把 TUI
  包进 tmux session（SSH 断线存活）。

### Config 新字段 — `core/src/config.rs` + `config/merge.rs`

- `Config.enable_tmux_session: Option<bool>`（默认 `None`=关闭）。
- `merge.rs` 增加该字段的 JSON 合并分支。

## 测试清单（9 条新增 unit，全部纯函数 <10ms）

| 文件 | 测试 | 断言 |
|------|------|------|
| `ts/actions.rs` | `classify_three_states` | Attached / Detached / Dead 三态枚举值正确 |
| `ts/actions.rs` | `sort_by_path_then_created_desc` | 完整排序序列：非 stopped 优先 → 路径升序 → 时间降序 |
| `ts/actions.rs` | `now_ms_is_milliseconds` | 时间戳在毫秒量级（>1e12） |
| `ts/display.rs` | `id8_truncates_to_eight_chars` | 截取 8 字符，含短串 / 空串边界 |
| `ts/display.rs` | `ms_to_secs_divides_by_1000` | 整除，含 0、余数截断边界 |
| `ts/display.rs` | `preview_of_prefers_preview` | 优先 preview，空白回退 title，双空边界 |
| `core/config.rs` | `enable_tmux_session_defaults_to_none` | 默认 `None`（零行为变更） |
| `core/config.rs` | `merge_into_applies_enable_tmux_session` | JSON 合并写入 `Some(true)` |
| `cli/tests/cli_parse.rs` | `ts_subcommand_parses_clean_flag` | `-c` 解析为 `clean=true`，其余 flag false |

## 全量验证

- `cargo test --workspace` — **1760 passed; 0 failed; 0 ignored**
- `cargo clippy --workspace --all-targets -- -D warnings` — **clean（零警告）**
- `cargo build --workspace` — **Finished dev profile**

## 技术债备注

- `crates/core/src/config.rs` 现 1002 行，超 800 行迭代上限。此为 **pre-existing**
  技术债（本会话前已 982 行，本次仅 +20），非本次引入。建议后续独立任务拆分
  `config/types.rs`。本次不阻塞。
