Commit: (working-tree, 基于 b21eccc)

# Thinking 定义 Step 边界

## Context

Turn 与 Step 的归属不能由工具完成时序或 provider message 边界推断。完整 Turn 由 Steps 与顶层 Say 配对；每个 Step 则由一段 Thinking 与下一段 Thinking 出现前的全部 function calls 配对。旧的“已有 finished call 后再启动工具就新建 Step”规则会把没有新 Thinking 的顺序调用错误拆开。

## Change Summary

- TUI live 在已有调用后收到新 Thinking 首帧时才创建下一 Step；单次 Thinking 后的顺序、并行和跨 provider round 调用都追加到当前 `N Function calls`。
- TUI replay 仍先按 assistant message 恢复临时调用批次，再把没有 Thinking 的相邻批次合回当前 Step；新 Thinking 保留为真实边界。
- SPA live 与 snapshot 使用同一规则：`reasoning_delta` 驱动 Step 新建，工具开始/结束只更新当前 Step 的调用列表。
- Turn → Step → Function call 的展开层级、Steps + Say 配对、running 动效和用户 disclosure 状态不变。

## Verification

- `cargo test -p opencoder-tui --lib`：1626 passed。
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings`：passed。
- `cargo test -p opencoder-web`：passed。
- `cd crates/web/spa && npm test -- --run`：151 passed。
- `scripts/build-spa.sh` 与 `scripts/check-spa-drift.sh`：生产 bundle 已重建且无漂移。

## Related Docs

- [Turn / Step / Function call 三级层级纠正](turn-step-function-call-hierarchy.md)
- [tui 模块](../../../agents/tui/index.md)
- [web 模块](../../../agents/web/index.md)
