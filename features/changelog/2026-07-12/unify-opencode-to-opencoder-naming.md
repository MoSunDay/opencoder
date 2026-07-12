Commit: (working-tree)

# 统一 opencode → opencoder 命名（运行时路径 / 配置文件 / bundle 扩展名）

## 背景

二进制名、workspace 包名、`~/.opencoder/skills`、`~/.opencoder/config.json`、clap name、`You are OpenCoder` 等已统一为 **opencoder**，但运行时数据目录、配置文件名、bundle 扩展名、DB 文件名等仍沿用 **opencode**（不带 r），两种命名混用。本次将所有用户可见的 `opencode` 路径/文件名统一为 `opencoder`。

## 变更

### 1. 运行时数据目录 `<data_local>/opencode/` → `opencoder/`
- `crates/tui/src/app_helpers.rs`、`crates/cli/src/lib.rs`（`tui_log_path`）、`crates/cli/src/session_cmd.rs`（`data_dir_for`）、`crates/web/src/lib.rs`（`data_dir_for`）中的 `push("opencode")` → `push("opencoder")`。
- DB 文件名 `opencode.db` → `opencoder.db`（`app.rs`、`session_cmd.rs`、`web/lib.rs`、`store_integration.rs` 注释）。

### 2. 配置候选路径与文件名（`crates/core/src/config.rs`）
- `<workdir>/.opencode/config.json` → `.opencoder/config.json`；`<workdir>/opencode.json` → `opencoder.json`。
- `~/.opencoder/opencode.json` → `opencoder.json`；删除冗余的 `~/.opencode/config.json` 候选（重命名后与 `~/.opencoder/config.json` 重复）。
- XDG `~/.config/opencode/` → `~/.config/opencoder/`。
- `save_target` / `save` 创建的项目配置文件 `opencode.json` → `opencoder.json`。
- 候选清单现为 5 项：`<workdir>/.opencoder/config.json`、`<workdir>/opencoder.json`、`~/.opencoder/config.json`、`~/.opencoder/opencoder.json`、`~/.config/opencoder/config.json`。

### 3. skills fallback（`crates/core/src/skill.rs`）
- 无 home 目录时的绝对回退 `.opencode/skills` → `.opencoder/skills`（与主路径同源）。

### 4. bundle 扩展名 `.opencode` → `.opencoder`
- `crates/store/src/bundle.rs`（模块 doc、`write_bundle`/`read_bundle` 注释、bad-magic 错误信息）、`crates/cli/src/lib.rs`（Export/Import clap doc）、`crates/cli/src/session_cmd.rs`（默认输出名 `{id}.opencoder`）。
- 注：二进制 magic `OPENCODR` 保持不变（格式兼容，非用户可见名）。

### 5. 注释 / 文档 / 用户可见字符串
- `crates/llm/src/tokens.rs`、`crates/session/tests/steer_followup.rs`、`crates/cli/src/session_cmd.rs`（`opencode models` 命令引用）、`crates/tui/src/model_menu/mod.rs`（`opencoder.json`）、`src/main.rs`（usage 提示 `opencoder "your prompt"`）。
- `agents/*/index.md`、`features/index.md` 同步更新（候选清单、命令名、路径）。

### 6. 测试同步
- `crates/core/tests/config_contract.rs`：所有项目配置文件名 `opencode.json` → `opencoder.json`（含 save 路径断言）。
- `crates/core/tests/skill_contract.rs`：`skills_dir` 断言简化为单一 `.ends_with(".opencoder/skills")`（回退已与主路径同源，不再有两分支）。
- `crates/web/tests/web_drain_contract.rs`：malformed config 文件名 → `opencoder.json`。

### 7. e2e 脚本同步
- `scripts/e2e/lib.py`：`AUTH_PATH` → `~/.local/share/opencoder/auth.json`；`seed_workdir` 写 `opencoder.json`。
- `scripts/e2e/cli_scenarios.py`：bundle 文件 `snake.opencode` → `snake.opencoder`。
- `scripts/e2e-glm.sh`：注释 `opencoder auth.json`。

### 8. `.gitignore`
- 新增 `.opencoder/` 与 `/opencoder.json`（保留旧条目以兼容过渡期残留文件）。

## 破坏性变更 / 迁移

| 旧路径 | 新路径 |
|--------|--------|
| `~/.local/share/opencode/<hash>/opencode.db` | `~/.local/share/opencoder/<hash>/opencoder.db` |
| `~/.local/share/opencode/tui.log` | `~/.local/share/opencoder/tui.log` |
| `~/.local/share/opencode/auth.json` | `~/.local/share/opencoder/auth.json` |
| `<workdir>/opencode.json` | `<workdir>/opencoder.json` |
| `<workdir>/.opencode/config.json` | `<workdir>/.opencoder/config.json` |
| `~/.opencode/config.json` | `~/.opencoder/config.json`（旧候选已删除） |
| `*.opencode` bundle 文件 | `*.opencoder` |

迁移：复制/重命名旧路径下的文件到新路径即可（`cp -r ~/.local/share/opencode ~/.local/share/opencoder`，并 rename DB 文件）。`~/.opencoder/config.json` 不受影响（本就正确）。

## 不在本次范围（保持不变）

- **crate 包名** `opencode-core`/`-cli`/`-llm`/`-session`/`-store`/`-tui`/`-web` 及所有 `use opencode_*` —— 内部实现细节，不影响用户可见二进制名 `opencoder`。
- **环境变量** `OPENCODE_MODEL`/`OPENCODE_SMALL_MODEL`/`OPENCODE_CONTEXT_LIMIT` —— 未在本次统一范围。
- **bundle magic** `OPENCODR` —— 二进制格式标识，保持兼容。

## 测试覆盖

`cargo build` + `cargo test` 全绿。`python3 -m py_compile scripts/e2e/*.py` 通过。
