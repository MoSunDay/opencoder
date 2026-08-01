# 评审修复轮：research/web_extract 测试拆分 + index.md 并发漏入还原

## Summary

处理上一轮评审的 2 个阻塞项：
1. **文件行数合规**：`research.rs`（455 行）与 `web_extract.rs`（479 行）超新增文件 400 行上限
   （fix 轮 rustfmt 扩容所致）。按仓库既有 `ssh_pty.rs`/`ssh_pty_tests.rs` 的
   `#[cfg(test)] #[path]` 约定，把测试块拆到同级 `research_tests.rs` / `web_extract_tests.rs`。
2. **范围外脏改动排除**：commit ebb0263 的 `agents/session/index.md` 混入并发 resume-replay
   任务的 `resume::resume` / `resume_and_replay` / `replay_cancelled_tasks` 三行文档重写
   （replay_timeout 300s / CancellationToken / `tests/resume_replay_timeout.rs` 等），
   还原为父提交版本，仅保留本 scope 的能力门控工具集行。

## Changes

### `crates/session/src/tools/research.rs`（455 → 285 行）
- 测试块（`#[cfg(test)] mod tests`）拆至同级 `research_tests.rs`（170 行，新文件），
  `research.rs` 以 `#[cfg(test)] #[path = "research_tests.rs"] mod tests;` 引用。
- 纯函数/tool 实现零改动，模块层级不变（`research::tests`），`use super::*` 可见性不变。

### `crates/session/src/tools/web_extract.rs`（479 → 333 行）
- 测试块拆至同级 `web_extract_tests.rs`（146 行，新文件），同上约定。

### `agents/session/index.md`
- 还原第 24–26 行 `resume::resume` / `resume_and_replay` / `replay_cancelled_tasks`
  文档至父提交版本（移除并发任务漏入的 replay_timeout/CancellationToken/测试引用）；
  对父提交的净 diff 仅剩本 scope 的「能力门控工具集」行（web_extract/research/
  chrome_headless 四引擎描述）。

## 测试清单（拆分纯搬运，测试本体零改动）

- 全部既有测试原样保留：`research::tests::*`（serp_url 四引擎 / merge_results /
  slugify / build_report / write_report 豁免测试 / research_smoke ignored）、
  `web_extract::tests::*`（九 profile 抽取 / 噪声剔除 / 通用回退 / format_article /
  tool execute）——仅文件位置从内联 tests 模块移至同级 `*_tests.rs`。

## 回归

- `cargo test --workspace`：1526 passed / 0 failed / 1 ignored（当次实跑）
- `cargo test -p opencoder-session --lib`：223 passed / 0 failed / 1 ignored（当次实跑）
- `cargo test -p opencoder-session --features browser --lib`：227 passed / 0 failed / 1 ignored（当次实跑）
- `cargo clippy --workspace --all-targets -- -D warnings`：零 error / 零 warning
- `cargo build --workspace`：干净 Finished
