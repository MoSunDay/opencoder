Commit: (working-tree, pre-initial-commit)

# config_contract: 6 个测试补齐 HOME/XDG 隔离，消除全局配置泄漏级联

## 背景

`crates/core/tests/config_contract.rs` 中 6 个测试在调用 `Config::load(dir)` 前只创建
了工作区 tempdir，但**未隔离 `HOME`/`XDG_CONFIG_HOME`**。`Config::load` 会按
`config_candidates()` 合并所有候选文件（含 `~/.opencoder/config.json` 与
`$XDG_CONFIG_HOME/opencode/...`），因此开发机真实的全局配置被深合并进来：

- `braces_api_key_resolves_env_var` 期望 `{ZHIPU_API_KEY}` 解析为 `secret-value-123`，
  实际拿到 `~/.opencoder/config.json` 里的真实 key → 断言失败 → panic 时持有共享
  `ENV_LOCK` → mutex 被毒化 → 其余 23 个测试全部 `PoisonError` 级联失败。

与 0721 提交（`capabilities_and_tools.rs`）修复的是同一类问题，但 `config_contract.rs`
当时未覆盖。

## 变更

### `crates/core/tests/config_contract.rs`（+6 / -6）

将 6 个缺少隔离的测试里的 `let dir = tempfile::tempdir().unwrap();` 替换为已有的
`isolated_home()` 辅助函数（与同文件其余 17 个测试一致）：

| 测试 | 作用 |
|------|------|
| `merge_project_file_overrides_defaults` | 项目文件覆盖默认值 |
| `env_overrides_project_file` | 环境变量覆盖项目文件 |
| `cache_salt_env_override` | cache_salt 环境覆盖 |
| `braces_api_key_resolves_env_var` | `{VAR}` 解析（级联源头） |
| `defaults_when_no_config_present` | 无配置时的默认值 |
| `reserved_saturates_against_context_limit` | reserved 饱和保护 |

`isolated_home()` 把 `HOME` + `XDG_CONFIG_HOME` 指向独立 tempdir，`config_candidates()`
里的全局候选 `.exists()` 均为 false，从而只合并测试写入的项目文件。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 项目文件覆盖默认值 | `merge_project_file_overrides_defaults` | `crates/core/tests/config_contract.rs` |
| 环境变量覆盖项目文件 | `env_overrides_project_file` | 同上 |
| cache_salt 三态 + 覆盖 | `cache_salt_env_override` | 同上 |
| `{VAR}` 花括号解析 | `braces_api_key_resolves_env_var` | 同上 |
| 无配置默认值 | `defaults_when_no_config_present` | 同上 |
| reserved 饱和 | `reserved_saturates_against_context_limit` | 同上 |

- `cargo test -p opencoder-core --test config_contract` → **25 passed / 0 failed**
  （修复前 1 passed / 24 failed）
- `cargo build -p opencoder-core` → Finished
- `cargo clippy -p opencoder-core --all-targets -- -D warnings` → Finished, 0 警告
  （当次实跑）

> 本变更仅触及 `crates/core/tests/config_contract.rs`（测试代码，零生产改动），故以
> in-scope crate gate 为准（当次实跑）：
> `cargo test -p opencoder-core --test config_contract` → **25 passed / 0 failed**（见上）。
> `cargo build -p opencoder-core` → Finished。
> 注：本改动仅替换 `tempdir()` → `isolated_home()`（与同文件 17 个既有测试一致），
> 不可能引入 clippy lint。`cargo clippy -p opencoder-core --all-targets -- -D warnings`
> → Finished, 0 警告（当次实跑）。
> 注：`cargo test --workspace` 当前含大量与本改动无关的预存 WIP 文件
> （tui/session/cli、chrome-headless、ssh-pty、db_lock 序列化等），其结果不反映本次提交，
> 故不作为本变更的验收依据；提交时仅 stage `config_contract.rs`。

## Gate

| 项 | 变更前 | 变更后 |
|----|--------|--------|
| config_contract 通过数 | 1 / 24 失败 | 25 / 0 失败 |
| ENV_LOCK 毒化 | 是（级联 23） | 否 |

## Impact Surface

- 向后兼容：是（仅测试代码，不改生产行为）。
- 行为变更：无（`Config::load` 语义不变，仅测试不再读取真实全局配置）。
- 不受影响：生产代码、其它 crate。
