Commit: (working-tree, pre-initial-commit)

# 统一 data_dir_for：CLI/TUI/Web 三端数据目录解析归一到 core

## 背景
`data_dir_for(workdir)` 此前有三份**互相漂移**的实现：
- **CLI**（`crates/cli/src/session_cmd.rs`）：`DefaultHasher` + `canonicalize` + 字符串哈希。
- **Web**（`crates/web/src/lib.rs`）：FNV-1a 手写哈希，**不** canonicalize，输出宽度 16。
- **TUI**（`crates/tui/src/app_helpers.rs`）：`DefaultHasher`，**不** canonicalize，对 `Path` 直接 `.hash()`（平台相关）。

三端对同一 workdir 可能算出**不同的** on-disk 数据目录，导致「在一个进程创建的 session 在另一个进程不可见」——表现为 `session not found` 与 `opencode[exited]` 脱管。本变更把唯一规范的算法提到 `opencoder_core::data_dir`，三端统一 re-export。

## 变更

### 统一到 core
- **`crates/core/src/data_dir.rs`**（新增，82 行）：`data_dir_for(workdir)`——`<data_local>/opencoder/<hash>`，`hash` 为 `DefaultHasher` 作用于 `canonicalize(workdir)` 的字符串形式（canonicalize 失败则回退原路径）。含 4 个单测：确定性、trailing-slash 折叠、symlink 解析、路径区分。
- **`crates/core/src/lib.rs`**：`pub mod data_dir;` + `pub use data_dir::data_dir_for;`。
- **`crates/cli/src/session_cmd.rs`**：`open_store` 改用 `opencoder_core::data_dir_for`；删除本地 `data_dir_for` 与其测试（已在 core 覆盖）。签名 `&PathBuf` → `&Path`。
- **`crates/tui/src/app_helpers.rs`**：删除本地 `data_dir_for`，改为 `pub(crate) use opencoder_core::data_dir_for;`。
- **`crates/web/src/lib.rs`**：删除 `data_dir_for`/`hash_of`（FNV），改为 `pub use opencoder_core::data_dir_for;`；测试改为确定性 + 路径区分断言。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 确定性 | is_deterministic | crates/core/src/data_dir.rs |
| trailing-slash 折叠 | canonicalizes_trailing_slash | crates/core/src/data_dir.rs |
| symlink 解析 | resolves_symlinks | crates/core/src/data_dir.rs |
| 路径区分 | distinguishes_different_paths | crates/core/src/data_dir.rs |
| web 确定性 | data_dir_for_is_deterministic | crates/web/src/lib.rs |
| web 路径区分 | data_dir_for_distinguishes_paths | crates/web/src/lib.rs |

- 全量回归：`cargo test --workspace` → 全绿
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：data_dir.rs 82 ≤ 400

## Impact Surface
- CLI / TUI / Web 三端对同一 workdir 解析出**完全相同**的 on-disk 数据目录——跨进程 session 可见性修复。
- **行为变化**：Web 端从 FNV 切到 `DefaultHasher`+canonicalize，TUI 端新增 canonicalize——已有本地数据目录路径会变（一次性迁移影响；unify 后稳定）。
- 不影响：session runner / store trait / LLM 边界。

## Related Docs
- [agents/core](../../agents/core/index.md)
- [agents/cli](../../agents/cli/index.md)
