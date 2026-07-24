Commit: (working-tree, pre-initial-commit)

# fix(tui): serialize sys_tokens tests with HOME-mutating tests to close race

## 背景
`sys_tokens_counts_system_prompt` 与 `sys_tokens_skill_body_dominates_skill_name`
两个测试调用 `sys_tokens_for`，该函数内部对 `home_dir()` 做**两次独立读取**
（`build_system` 与 `global_instructions_text` 各一次）。

测试套件默认多线程并发运行。同文件下方的 `apply_skill_tokens_*` 系列测试通过
`with_home` helper 在同一 `APPTEST_HOME_MUTEX` 保护下临时改写 `HOME` 环境变量。
当某个 `with_home` 测试恰好在 `sys_tokens_for` 的两次 `home_dir()` 读取之间
改写了 `HOME`，两次读取取到不同的家目录，导致 global-subtraction 下溢为 0，
使 `sys_tokens_counts_system_prompt` 的 `base > 0` 断言偶发性失败（test flake）。

## 变更
### sys_tokens 测试纳入 HOME 互斥锁
- **`crates/tui/src/app_tests.rs`** `sys_tokens_counts_system_prompt` (≈518 行)：
  开头获取 `APPTEST_HOME_MUTEX.lock()` guard，与 `with_home` 测试串行化。
- **`crates/tui/src/app_tests.rs`** `sys_tokens_skill_body_dominates_skill_name`
  (≈547 行)：同样获取 guard，消除同类竞态窗口。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 系统提示 token 计数不受并发 HOME 改写影响 | `sys_tokens_counts_system_prompt` | app_tests.rs |
| skill body 优于 name 的计数对比稳定 | `sys_tokens_skill_body_dominates_skill_name` | app_tests.rs |

- 全量回归：`cargo test --workspace` → 921 passed; 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净编译
- 行数：app_tests.rs 1213（既有测试聚合文件；本次仅 +6 行，未引入新职责边界）

## Impact Surface
- 仅消除测试偶发失败（flake），无运行时行为变化。
- 不影响：CLI / Web / session / store / LLM 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
