Commit: (working-tree, pre-initial-commit)

# feat(model): unify model-switch default = session-only across TUI/Web/CLI

## 动机

三个入口（TUI `/model`、Web `POST /sessions/:id/model`、CLI）切换模型时，
**默认就写 `opencoder.json`**（全局默认）。这与"先试再用"的直觉相悖——
用户在 `/model` 列表按一下 Enter，磁盘配置就被永久改写；要回退得手动编辑
JSON。本次统一语义：**默认 session-only（只改内存 + 会话行，不写盘）**；
只有明确确认（`y` / `persist_default=true` / `config set`）才落盘全局默认。

## 变更

### 1. TUI `/model` 菜单（`crates/tui/src/model_menu/`）

- **`state.rs`**：新增 `ModelOutcome::SaveSessionOnly(serde_json::Value)` 变体——
  应用 merge-patch 到内存 config **但不调 `Config::save`**。
- **`list.rs`**：列表 Enter 不再立即保存，而是进入两步确认态
  `confirm_save_default: Option<Value>`：
  - `y`/`Y` → `ModelOutcome::Save`（`Config::save` 写回 opencoder.json）。
  - `n`/`N`/Enter/Esc → `ModelOutcome::SaveSessionOnly`（内存热换 + 派发
    `ReloadConfig`，worker 把新 model 写进 session store 行，resume 即生效）。
- **`view.rs`**：确认态时浮层标题改为
  ` /model — SAVE AS DEFAULT? y=global, n/Enter=session-only `。
- **`app_loop.rs`**：`handle_model_outcome` 的 `SaveSessionOnly` 分支抽出为
  `model_session_switch::switch_session`（新建模块，控制 `app_loop.rs` ≤800 行）：
  更新 `config.model` → 重建外层 `client`（新 `/task` 会话也用上新 endpoint）→
  派发 `ReloadConfig` → 推青字 `switched (session only)` marker。

### 2. Web `POST /sessions/:id/model`（`crates/web/src/api.rs`）

- `ModelBody` 新增 `persist_default: bool`（`#[serde(default)]` = false）：
  - `false`（默认）→ 只更新 store meta + `handle.overrides.model`（session-only）。
  - `true` → 额外 `Config::save(&workdir, {model})` 写全局默认；失败返回 500。

### 3. CLI `config set <model>`（`crates/cli/src/{lib,session_cmd}.rs`）

- 新增 `ConfigSub::Set { model }` 子命令 → `Config::save` 写回 opencoder.json
  （headless/脚本化设默认模型的入口）。

## 测试覆盖

新增 **12** 个测试函数，删除 **1** 个（`list_enter_switches_provider`，旧断言
的单步 Enter→Save 行为已不存在，被 5 个覆盖两步确认的测试取代），净 **+11**
（1065 → 1076 passed）。

| 入口 / 功能 | 测试名 | 文件 |
|-------------|--------|------|
| TUI 列表 Enter 进入确认态 | `list_enter_arms_save_default_prompt` | `crates/tui/src/model_menu/tests/provider_tests.rs` |
| TUI `y` → Save 写盘 | `list_enter_then_y_saves_as_default` | `crates/tui/src/model_menu/tests/provider_tests.rs` |
| TUI `n` → SaveSessionOnly | `list_enter_then_n_is_session_only` | `crates/tui/src/model_menu/tests/provider_tests.rs` |
| TUI Enter → SaveSessionOnly | `list_enter_then_enter_is_session_only` | `crates/tui/src/model_menu/tests/provider_tests.rs` |
| TUI Esc → SaveSessionOnly | `list_enter_then_esc_is_session_only` | `crates/tui/src/model_menu/tests/provider_tests.rs` |
| TUI SaveSessionOnly 全链路（热换 client + ReloadConfig + 不写盘 + marker） | `handle_model_outcome_session_only_skips_disk_write` | `crates/tui/src/app_loop_session_only_tests.rs` |
| Web persist_default=true 写盘 | `post_model_persist_default_writes_config` | `crates/web/tests/web_contract.rs` |
| Web 默认 session-only 不写盘 | `post_model_default_is_session_only_no_disk_write` | `crates/web/tests/web_contract.rs` |
| Web persist_default 恶意模型值返 500、不写盘 | `post_model_persist_default_malformed_returns_500` | `crates/web/tests/web_contract.rs` |
| CLI config set 解析 | `config_set_subcommand_parsed` | `crates/cli/tests/cli_parse.rs` |
| CLI config set bare model 解析 | `config_set_bare_model_parsed` | `crates/cli/tests/cli_parse.rs` |
| CLI config set 写盘 + reload | `config_set_persists_model_to_disk` | `crates/cli/src/session_cmd.rs` |
