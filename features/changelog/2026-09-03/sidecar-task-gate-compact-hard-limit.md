Commit: (working-tree, 基于 2677992)

# sidecar `task` 封堵 + 手动压缩硬上限熔断（sidecar 豁免）

## Context

两条独立于 Say 配对工作的 session 缺陷（bugfix brief round 2 的 #2/#7；同 brief 的 #4/#6 已随 [N Steps 与 Say 成对](say-pairs-steps-all-surfaces.md) 落地；#3/#5 中 SPA seq 竞态的一半已随该篇 chat.jsx/chat.dom.test.jsx 落地，SSE resync 去重仍未做，保持 open）：

1. **sidecar 借 `task` 外泄写能力**：`execute_call_with_timeout` 对 `task` 早于通用门返回，sidecar 可 spawn 全写能力 build 子代理改仓库——5f06260 给了只读 bash 门，却留下这个更大的口子。
2. **手动压缩越过上下文窗口静默劣化**：`compaction.auto=false` 时 `should_compact` 恒 false，transcript 超模型窗口后每问必 400/劣化且无从得知该拉哪个杆。

## Change Summary

- `runner/execute.rs`：`task` spawn 前先过 `bash_guard::gate`（sidecar 全拒、plan 拒写向子代理，拒文与通用门一致），先于任何 child session 创建。
- `compaction/mod.rs`：新增 `exceeds_hard_limit`（estimated 或 reported ≥ `context_limit()`）+ `MANUAL_COMPACT_HINT`；`runner/mod.rs` 主循环在 `compaction.auto=false` 且超硬上限时以可操作 Error 终止 run。
- **sidecar 豁免**（`runner/mod.rs`）：sidecar transcript 是借用的父快照且 `auto` 被强制关——gate 会用 sidecar 内不可执行的 "/compact" 提示永久失败每个问题，故 `agent.name != "sidecar"` 才熔断，让真实 provider 上下文错误诚实浮出。
- 事故修复：此前追加 sidecar 测试时误删 `#[cfg(test)] #[path = "execute_timeout_tests.rs"] mod timeout_tests;`，5 例超时路由测试静默消失——已恢复声明并回归。
- `bash_guard.rs`：归位 5f06260 遗留的孤儿 doc 注释（gate 的文档误挂在 `GateRule` 上且残留重复片段），workspace clippy 归零。

## Test list (rules/02)

- `runner::execute::tests::sidecar_cannot_farm_mutations_out_via_task`（新增）。
- `compaction::tests::exceeds_hard_limit_fires_when_transcript_passes_the_model_window` / `..._stays_false_with_headroom`（新增）。
- `tests/sidecar_loop.rs::sidecar_answers_when_parent_snapshot_exceeds_hard_limit`（新增，豁免回归）。
- `runner::execute::timeout_tests::*` ×5（恢复声明后回归）。
- `cargo test --workspace`：253 块 / 3997 passed / 0 failed；`cargo clippy --workspace --all-targets` 0 warning；fmt 通过。
