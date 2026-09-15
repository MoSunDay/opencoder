# agent 节点启动 seed 内置技能包（消除最后一个会话宿主的资产不传播缺口）

## 背景

- 承接 review 精简输出契约（`f107d644`）的遗留 TODO：将 `seed_builtin_skills()` 接回启动路径。
- 核查结论：主二进制 `src/main.rs:44` 自 `eda4ff87`（07-21）起就在全部分发臂之前调用 `seed_builtin_skills()` + `seed_dep_gated_skills()`，headless/TUI/ts/`daemon --server` 全覆盖——前序 brief「无任何生产启动路径调用」系探索时只 grep `crates/` 漏掉根 `src/`，该前提过时。
- 真实残余缺口只剩 **`opencode-agent`**：worker 节点跑真实会话 runner（`crates/agent/src/main.rs` → `Worker::open` → `harness::initialize` + `ensure_drain`），技能从节点本地 `~/.opencoder/skills` 解析，但二进制启动从不 seed，节点上的旧资产不会随升级传播。

## 变更

- **`crates/agent/src/main.rs`**：`run()` 中 `init_logging()` 之后、离线短路与 `Worker::open` 之前，调用 `opencoder_core::seed_builtin_skills()` + `seed_dep_gated_skills()`——与主二进制完全同策略（增量、best-effort、update-on-drift、dep-gated never-clobber、无 home 时 warn 跳过），节点升级即自动收敛漂移资产。
- **`tests/skill_seed_startup_wiring.rs`**（根包新增）：源码契约测试锁定两个会话宿主的启动接线——必须包含两个 seed 调用（精确调用串，注释提及不计数），且 seed 必须先于首个会话宿主语句（主二进制锚 `run_headless`，agent 锚 `Worker::open`）。存源级断言的原因：接线位于 `fn main`，无可调用缝，不为可测性重构生产入口。
- **`agents/agent/index.md`**：关键路径补一行启动 seed 语义。

## 回归（当次实跑）

- `cargo test -p opencoder --test skill_seed_startup_wiring`：2 passed / 0 failed。
- `cargo build --workspace --bins`：全部 bins 编译通过（agent 接线为纯启动调用，无行为分支）。
- `cargo test --manifest-path crates/agent/Cargo.toml`：5 passed / 0 failed。
- **workspace 全量回归**（`cargo test --workspace --no-fail-fast`）：389/391 套件 ok，**5183 passed / 2 failed**，实跑于隔离 `CARGO_TARGET_DIR`（并行会话持续占用共享 `target/`）。
- 2 个失败均为 **HEAD 预存**、与本变更无因果：①`crates/worker/tests/dag_live_logs.rs:71` wasm 活日志 15s 超时（stash 掉本变更后同点复现，30.96s）；②`crates/worker/tests/runner_dispatch.rs:90` Runner 拒绝断言（期望 400）。worker 测试进程内跑 `Worker`，不链接也不 spawn `opencode-agent` 二进制。已列入问五跟进。
- 环境备注：本会话中 cargo `-p <member>` 包名匹配对全部带连字符成员失效（`cargo metadata`/`--manifest-path`/`--workspace` 均正常），故全部命令改用 manifest-path / --workspace 形式。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `local_binary_seeds_skills_before_any_dispatch`（新增） | integration（源码契约） | 主二进制含 `seed_builtin_skills();` + `seed_dep_gated_skills();` 精确调用串，且先于 `run_headless` 首个分发臂 |
| `agent_binary_seeds_skills_before_worker_opens`（新增） | integration（源码契约） | agent 二进制含两个 seed 精确调用串，且先于 `Worker::open`（节点会话运行时起点） |
