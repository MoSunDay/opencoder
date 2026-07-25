Commit: (working-tree, pre-initial-commit)

# fix(session/glob): 彻底阻止 `**` 经软链接的分支递归死循环

## 背景

`grep` 工具的软链接循环修复（commit `3015982`，canonical-path `HashSet`
去重）是完整的，但同一类 bug 在 `GlobTool` 里完全没有修复。现有的
`glob_survives_self_referencing_symlink` 测试给了虚假的安全感：

- `glob` crate 0.3.3 的 `**` 会跟随软链接（`from_dir_entry` 用
  `fs::metadata` 解析 symlink→dir 并下降），且 `MatchOptions` 没有
  `follow_links` 字段，无法关闭。
- `GlobTool` 直接裸调 `glob::glob()`，只在事后把输出截断到 500 条，没有任何
  重入保护。
- 旧测试只造了一个自引用软链接 `symlink(".", "loop")`——这是线性链（深度约
  40，靠内核 `ELOOP` 秒过）。但同一目录里只要两个及以上互相/自引用软链接，
  就是分支递归 → 2^depth 条路径 → 实质死循环 / 巨量 IO。这就是"还没解决"。

## 变更

### `crates/session/src/tools/glob.rs`（重写遍历逻辑）
不再裸调 `glob::glob()` 让它无边下降。改为：
- 用 `glob::Pattern::new(full_pattern)` 编译模式，自行 `read_dir` 递归遍历，
  每条完整路径用 `compiled.matches_path_with(path, opts)` 判定是否匹配
  （保留 glob 匹配语义，不重写匹配逻辑）。
- **两路分发**：`is_literal(full_pattern)`（无 `*?[`）走字面路径分支，直接
  `path.exists()` 返回该路径（文件或目录），与 `glob::glob()` 字面模式语义一致；
  否则进入 `walk` 递归。这是实现中发现并补上的——纯字面模式（如 `a.rs`）会让
  `literal_root` 落到文件本身，`read_dir` 必失败，必须单独处理。
- 新增纯函数 `walk(dir, pattern, opts, dir_only, out, visited, seen)`：
  - `seen: HashSet<PathBuf>` + `visited: u32`，与 `grep.rs::walk` 完全同构。
  - 每个目录先 `canonicalize()` → `seen.insert` 命中即 `return`，打破软链接
    循环（与 `grep.rs:77-81` 一致）。
  - `MAX_VISITED = 50_000` 兜底（canonicalize 无法覆盖的分支，如权限拒绝）。
  - `RESULT_CAP = 500`，沿用旧 `take(500)`。
  - `dir_only`（`is_doublestar_terminal`：模式末段恰为 `**`）→ 只收目录。
    经验证 `matches_path_with` 对 `**/*.rs`、`*.rs`、`sub/**/*.rs`、字面模式、
    `.hidden` 与 `glob::glob()` 逐元素相等；仅末段 `**` 的"仅目录"语义需此开关补齐。
  - 剪枝名单与 grep 同步：`.git | node_modules | target | dist | .next | .cache`。
- `MatchOptions`：`require_literal_separator = true`、`case_sensitive = true`、
  `require_literal_leading_dot = false`，确保 `matches_path_with` 语义与旧
  `glob()` 一致（`*` 不跨 `/`）。已在独立探针里对 12 种模式做了逐元素比对，
  全等。
- 新增 `literal_root(full_pattern)`：取模式最长字面（非通配）前缀作为遍历根，
  对 `src/**/*.rs` 只从 `<base>/src` 起走，避免无谓全树扫描。
- 纯函数式，无 `class`，符合仓库规则。文件 211 行，远低于 400 行限制。

### `crates/session/tests/tools_contract.rs`（回归测试，`#[cfg(unix)]`）
- `glob_survives_multiple_self_referencing_symlinks`（新增）：同一目录放 `a -> .`、
  `b -> .` 两个自循环 + `target.rs`，断言 `**/*.rs` 在 **< 5s** 内返回、包含
  `target.rs`、未撞 500 上限（旧实现会挂死 / 爆炸）。这是真正的回归点。
- `glob_matches_normal_tree_parity_with_crate`（新增）：一棵无 symlink 的混合
  树，断言新实现与直接 `glob::glob()` 结果集**逐元素相等**，防止
  `matches_path_with` 语义漂移（`.hidden.rs`、`**`、`dist` 剪枝均覆盖）。
- 保留旧 `glob_survives_self_referencing_symlink`。
- 保留 `glob_tool_matches_pattern`。

## 测试清单（rules/02-regression-gate）
- `cargo test -p opencoder-session --test tools_contract glob` → 4/4 绿，双自循环
  用例 < 5s（实测 `glob_survives_multiple_self_referencing_symlinks` < 1s）。
- `cargo test -p opencoder-session --test tools_contract` → 19/19 绿。
- `cargo test -p opencoder-session --lib tools::glob` → 3 个单测绿：
  `literal_root_stops_at_first_glob_component`、
  `is_doublestar_terminal_detects_trailing_recursive`、
  `is_literal_detects_wildcards`。
- `tools_contract::glob_matches_normal_tree_parity_with_crate`：无 symlink 混合树
  上对 8 种模式（`**/*.rs`、`*.rs`、`sub/**/*.rs`、字面、`.hidden`、`**/*`、
  末段 `sub/**`）与 `glob::glob()` 逐元素相等。
- 真实仓库子树 parity（throwaway 探针）：`crates/llm/src/**/*.rs` 11/11、
  `crates/**/*.rs` 196/196、`**/Cargo.toml` 39/39、
  `crates/session/src/tools/*.rs` 18/18，全等。
- 全量回归 `cargo test -p opencoder-session -p opencoder-core -p opencoder-llm
  -p opencoder-store` → 全绿，0 失败。
  - 注：`opencoder-tui` 当前 working-tree 存在独立的 WIP 编译错误，与本修复
    无关，无法纳入 `--workspace` 全量；本修复不触及 tui。

## 风险与对齐
- canonicalize 每目录一次 IO：与 `grep.rs` 已接受的开销一致。
- `matches_path_with` vs `glob()` 语义：用 parity 测试兜住，已确认 `**`、
  `require_literal_separator`、`.hidden` 行为一致。
- 与 `grep.rs` 修复范式完全一致（canonical `seen` + visited cap），纯函数式，
  无 class，符合仓库规则。
