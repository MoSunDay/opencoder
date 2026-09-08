# server: 日志引导模块去 cli 旧名（评审顺手项）

## 背景

上一轮评审（对象 commit `c853064e`）通过（READY），遗留唯一可当场落地的顺手项：`crates/server/src/main.rs` 内部模块 `opencoder_cli_compat` 是 cli→local 更名前的残留命名——该模块实为服务端自带的极简日志引导（共享版在 `opencoder-local` crate），与 "cli compat" 无关。

## 变更

- `mod opencoder_cli_compat` → `mod logging`（私有模块重命名，仅 main.rs 内 2 处引用同步）
- 模块 doc 注释中过期的 "the cli crate owns the shared one" 更正为 "the local crate"
- 零行为变化：纯命名清理，无 API / 路由 / 配置改动

评审其余 TODO 维持原判不动：flake 为观察项（CI 监控）；server 面 gap（节点注销路由、team topics 半区、bundle 导入导出、集群级 env/config）留待后续迭代。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| token 解析（flag 优先/互斥/文件 trim） | `resolve_token_param_wins` / `token_flags_are_mutually_exclusive` / `token_file_is_trimmed_and_empty_file_is_rejected` | `crates/server/src/main.rs`（内嵌） |

本轮零新增功能，测试数与基线持平（无测试被删改）。

## 验证

- 全量回归：`cargo test --workspace --no-fail-fast` → 346 个测试目标全绿（4904 passed / 0 failed / 5 ignored，ignored 均为预置 manual 用例）
- Lint：`cargo clippy --workspace --all-targets -- -D warnings` → EXIT=0，零警告
- fmt：`cargo fmt --all --check` → EXIT=0
- 构建：`cargo build --workspace` → EXIT=0
