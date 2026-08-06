Commit: (working-tree, pre-initial-commit)

# 拆分 config.rs：env 机制与测试外置，回归 800 行门控

## 背景
`crates/core/src/config.rs` 经 tool-agent/browser 移除后仍为 **964 行**，超出 800 行
迭代文件上限（全局工程要求）。env/discovery 机制与内联 `mod tests` 混在单文件内，
职责过载。本次按既有 `config/{autopilot,merge}.rs` 子模块约定做**纯机械、行为不变**
的拆分，使 config.rs 回到门控以内。

## 变更

### 提取 `config/env.rs`（新增，155 行）
config 发现 + 环境覆盖 + 线程本地隔离机制整体外置：
- `looks_like_env_var` / `scoped_config_home` / `ScopedConfigHome`（保持 `pub`）。
- `ISOLATION` thread-local + `isolated_home` / `config_home_dir` / `config_xdg_dir`（私有）。
- `env_get` / `config_candidates` / `apply_env` 提升为 `pub(super)`，供 `impl Config` 经
  `env::` 前缀调用；`resolve_env` 保持 `pub(super)`。
- 沿用 `use super::Config;` + `pub use env::{looks_like_env_var, scoped_config_home, ScopedConfigHome};`
  的 re-export 约定，`lib.rs` 的 crate 根重导出名单不变。

### 提取 `config/tests.rs`（新增，241 行）
内联 `#[cfg(test)] mod tests { … }` 原样外置为 `config/tests.rs`，声明改为
`#[cfg(test)] mod tests;`。子模块对父私有项的 `super::` 访问不变，**零断言/路径改动**。

### `config.rs` 收尾
- 顶部新增 `mod env;` 与对应 `pub use`。
- 删除已迁移的 env 代码块；`impl Config` 5 处调用点改 `env::` 前缀
  （`config_candidates`×2、`apply_env`、`env_get`、`resolve_env`）。
- `config/merge.rs`：2 处 `super::resolve_env` → `super::env::resolve_env`。
- **config.rs：964 → 579 行**（≤ 800 ✅）。

### memory 文档 repair-on-touch
- `agents/session/index.md` L29：工具集描述由已删除的 web_read/web_extract/serp/research
  改为现行 8 个无门控内建工具（bash/read/view_image/edit/search/ls/task/ssh_pty）+ plan 模式
  schema 层 read-only 契约。其余 memory 文档无漂移。

## 测试覆盖（当次实跑）
- `cargo build --workspace` → Finished，0 error
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 warning
- `cargo test -p opencoder-core --lib config::` → 26 passed / 0 failed
- 全量 `cargo test --workspace` → 1895 passed / 0 failed（连跑 2 次均稳定，含 is_deterministic 抖动修复后）

## 附带修复：消除 `is_deterministic` 并行测试抖动

拆分后顺带发现并修复了一个**预存的并行测试 flake**：`config/tests.rs` 的
`save_handles_corrupt_and_empty_config_files` 此前用 `std::env::set_var("HOME", …)`
做进程级 HOME 隔离，与 `data_dir::tests::is_deterministic` 里的 `dirs::data_local_dir()`
读取在并行执行下构成 `getenv`/`setenv` 数据竞争（libc 层不安全），偶发使该确定性断言失败
（即 review 报告的「911/914/919 计数漂移」非确定性根因之一）。

- **修复**：改用 `scoped_config_home` 线程本地隔离（不触碰进程 env）——该机制本就是为取代
  这类不安全 `set_var` 而设计（见 `config/env.rs` 文档）；`save_target` 的候选解析行为不变。
- **验证**：`cargo test -p opencoder-core --lib` 并行连跑 6 次 → 每次 79 passed / 0 failed
  （修复前同条件偶发 `is_deterministic` 失败）。
- **残留风险（未修）**：`net.rs` 的代理测试仍用 `std::env::set_var` 设 `OPENCODER_PROXY` 等，
  同属 libc 不安全写入；但无跨模块可观测断言冲突（无测试断言 proxy 字段），未观察到失败。
  彻底修复需把 `effective_proxy`/`build_http_client` 的 env 读取经可注入缝（如 `env_get`）路由，
  属更大改动，标记为后续。

## Impact Surface
- 纯模块拆分，**无 API/行为变更**：`opencoder_core::{looks_like_env_var, scoped_config_home,
  ScopedConfigHome, Config, …}` 的公开面与序列化形态完全不变。
- 内部可见性：`env_get`/`config_candidates`/`apply_env` 由私有提为 `pub(super)`（仅在
  `config` 模块树内可见）。
- 行数合规：config.rs 579 / env.rs 155 / tests.rs 241，均 ≤ 800。
