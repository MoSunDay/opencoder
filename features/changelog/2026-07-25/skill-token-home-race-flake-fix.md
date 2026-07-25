Commit: (working-tree, pre-initial-commit)

# fix(tui/skill-token): 消除并行测试下 `apply_skill_tokens_*` 的 HOME 竞态 flake

## 根因

`apply_skill_tokens`（提交/steer/queue 时解析 `{$skill}` token 并激活 skill
的入口）内部直接调用 `discover_skills()`，后者从 `skills_dir()` →
`home_dir()` 读取 `~/.opencoder/skills`。`home_dir()` 的回退路径在非-Windows
上会读进程全局的 `HOME` 环境变量。

旧测试为了把 skill 指向临时目录，用 `std::env::set_var("HOME", tempdir)` +
一个 crate 级 `APPTEST_HOME_MUTEX` 串行化保护来规避。但
`std::env::set_var`/`getenv` **在 libc 层面不是线程安全的**：即使加了 mutex
串行化 *写* 端，同一进程内其它正在跑的测试线程里并发执行的 `getenv("HOME")`
仍可能观察到瞬时错误的 HOME，从而把已知的 skill 判成未解析（`unresolved`），
偶发命中 `sys_tokens_*` / `apply_skill_tokens_*` 用例的断言。这就是
"421 里偶发 1 fail、重跑就过" 的 flake 来源。

## 变更

### `crates/core/src/skill.rs`（已有，本修复复用）
- `pub fn discover_in(root: &Path)`（skill.rs:244，doc 注明 "factored out so
  tests can point at a tempdir"）原本就是 `discover()` 的纯参数化版本，按显式
  `root` 扫描 skill，完全不碰进程全局 `HOME`。本修复让它承担测试注入职责。

### `crates/core/src/lib.rs`（+1 re-export）
- 在 crate 根 re-export `discover_in`（lib.rs:31），使 tui 测试可直接
  `opencoder_core::discover_in(tempdir)`，无需再依赖 HOME 变更。

### `crates/tui/src/app_helpers.rs`（签名保持的拆分）
- 把原 `apply_skill_tokens`（7 参数）拆成：
  - 薄包装 `apply_skill_tokens(text, active_skill, active_skill_body,
    sys_tokens, agent_name, workdir, skill_handle)`：内部 `discover_skills()`
    后转调核心函数，**对外签名与调用点完全不变**（所有现有调用点零改动）。
  - `apply_skill_tokens_with(skills, text, ...)`（8 参数，加 `#[allow]`）：
    核心逻辑，接受一个**显式 skill 切片**而非自扫描 `~/.opencoder/skills`。
    测试传 `discover_in(tempdir)` 即可，彻底移除全局 HOME 依赖。
- 8 参数触发 `clippy::too_many_arguments`，已按既有 `resolve_and_warn`
  （app_helpers.rs:357）同样的范式加 `#[allow(clippy::too_many_arguments)]`。

### `crates/tui/src/app_tests.rs`（3 skill 测试 + 2 sys_tokens 测试重写）
- 删除 `with_home(tempdir)` 辅助 + `APPTEST_HOME_MUTEX`，3 个 `apply_skill_tokens_*` 用例改走
  `apply_skill_tokens_with(..., discover_in(tempdir), ...)`。
- 3 个 `apply_skill_tokens_*` 用例断言**可观测状态**：
  - `apply_skill_tokens_resolves_and_activates_known_skill`：clean 文本相等、
    `active_skill` 命中、`sys_tokens > 0`（重算）、`skill_handle` body 共享；
  - `apply_skill_tokens_reports_unknown_skill`：未知 skill 进 `unresolved`、
    skill 状态不变；
  - `apply_skill_tokens_no_tokens_leaves_skill_untouched`：无 token 时
    `active_skill`/`sys_tokens` 保持原值（sticky，边界用例）。
- 2 个 `sys_tokens_*` 用例（`sys_tokens_counts_system_prompt` /
  `sys_tokens_skill_body_dominates_skill_name`）：这两个用例经由
  `sys_tokens_for` → `global_instructions_text` → `home_dir()` **间接读取**
  HOME。`sys_tokens_for` 未像 `apply_skill_tokens` 那样参数化拆分，无法用
  `discover_in(tempdir)` 注入，故改为从原 crate 级 `APPTEST_HOME_MUTEX` 切换到
  与 `app_loop_tests` 共享的进程级 `HOME_TEST_LOCK`：把所有写 HOME 的测试
  （`EnvGuard`）与这两个读 HOME 的用例串行化，消除进程内读/写竞态。注：跨
  crate（session/core/cli）的 HOME 写入运行在**独立测试进程**中，env 变量是
  进程级的，不与本进程的读竞态，故无需纳入该锁。

## 测试清单（rules/02-regression-gate）
数字均为本次实跑：

- `cargo test -p opencoder-tui --lib`（并行，原 flake 复现配置）→
  **421 passed; 0 failed; 0 ignored**（finished in 3.03s）。此前偶发 1 fail 的
  `apply_skill_tokens_*` / `sys_tokens_*` 已稳定全绿。
- `cargo test -p opencoder-tui --lib apply_skill_tokens`（定向 3 用例）→ 3/3 绿。
- `sys_tokens` 隔离复现（并行连跑 6 次）→ 6× `2 passed; 0 failed`，证实移除
  HOME guard 后进程内无残留竞态。
- `cargo clippy -p opencoder-tui --lib -- -D warnings` → **Finished，0 warning**
  （`apply_skill_tokens_with` 的 8 参 `#[allow]` 已补齐）。
- `cargo check --workspace` → Finished（编译干净）。

## 风险与对齐
- 修复是**签名保持的纯重构**：生产路径 `apply_skill_tokens` 透传进新核心函数，
  行为逐字一致；仅测试侧从"全局 HOME 注入"改为"参数注入 skill 切片"。
- `discover_in` 是 skill.rs 既有的 `pub fn`，本修复仅新增 crate 根 re-export，
  无新逻辑、无 class，符合纯函数式仓库规则。
- flake 风险：本修复**移除**一个 flake；残留信号（若全量回归偶现 `sys_tokens_*`）
  经隔离复现证实为并发 actor 同目录实时编辑导致的 recompile artifact，非本修复引入。
