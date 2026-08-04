Commit: (working-tree, pre-initial-commit)

# 版本信息带上 git commit id

## 背景
此前版本字符串只来自 `[workspace.package] version`（`0.1.0`），通过 clap 自动
`--version` 暴露，既不含 commit 也不含 dirty 标记。开发态频繁改动时无法从版本号
判断它对应哪次提交、是否带未提交改动；运行中的 server 同样无版本可见。

## 变更

### A. 构建期捕获 git 元数据

**`crates/core/build.rs`**（新建，77 行）：构建时 `git rev-parse` 取 short/full
commit，`git diff --quiet` / `--cached --quiet` 判定 dirty，经
`cargo:rustc-env` 暴露 `OPENCODER_GIT_COMMIT` / `OPENCODER_GIT_COMMIT_FULL` /
`OPENCODER_GIT_DIRTY` / 预组装的 `OPENCODER_VERSION_LONG`。设 `rerun-if-changed`
指向 `.git/HEAD` 与 `.git/packed-refs`，HEAD 变化即重编译刷新 commit。非 git
构建（如 tarball）graceful 退化为 `"unknown"`，编译永不失败。`assemble` 镜像
`format_version`，由单测断言二者一致以防漂移。

### B. 版本常量与纯函数（core::version）

**`crates/core/src/version.rs`**（新建，81 行）：暴露 `VERSION` / `GIT_COMMIT` /
`GIT_COMMIT_FULL` / `VERSION_LONG` 常量与 `format_version()` / `long_version()` /
`is_dirty()` 函数。`option_env!` 用 `match`（const-stable），`is_dirty` 用运行时
`matches!`（`&str` 相等尚未 const-stable）。

### C. 三处版本表面接入

- **`crates/cli/src/lib.rs`**：clap `#[command(version, long_version = VERSION_LONG)]`。
  `-V` 仍输出短版本 `opencoder 0.1.0`；`--version` 输出 `opencoder 0.1.0 (125b34c)`
  或带 `-dirty`。
- **`crates/web/src/lib.rs`**：server 启动 banner（`println!` + `tracing::info!`）
  追加版本，运行中的 server 即可见 commit。
- **`crates/web/src/api.rs`**：`/api/health` 由 `{"ok":true}` 扩展为
  `{"ok":true,"version":"...","commit":"..."}`。

### D. 测试

- `core::version`（unit，inline）：`format_version` 的 clean/dirty/unknown 三态；
  `baked_long_version_matches_format_contract` 断言 build 期常量与纯函数一致；
  `long_version_carries_commit_id` 断言版本含 commit。
- `web_contract::health_ok_carries_version_and_commit`（integration）：读
  `/api/health` body，断言含 `version`/`commit` 且 version 含 commit id。

非 git 环境下 `commit` 退化但语义自洽（`(unknown)`）。

修复后：1830 tests / 0 failed / 1 ignored / 0 clippy warnings。
