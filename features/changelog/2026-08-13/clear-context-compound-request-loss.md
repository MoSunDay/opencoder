# `/act_clear_context <request>` 在保留计划分支下丢失请求

## 背景

`/act_clear_context review` 这类复合 clear_context 命令有两个代码路径：

1. **sentinel 路径**（无保留计划）：清空上下文后停止。
2. **preserved-plan 路径**（transcript 含 assistant 消息，`final_plan_text()`
   返回 `Some`）：清空上下文但保留计划，fall through 到 `run_loop` 执行计划交接。

在 preserved-plan 路径中，ClearContext 分支**无条件**执行
`user_text.clear()`，将复合命令的 trailing request（如 `"review"`）一并丢弃。
结果：用户输入 `/act_clear_context review` 后，上下文被清空、计划被保留，
但 `"review"` 既未记录为用户消息、也未传给 LLM 执行——请求静默丢失。

sentinel 路径（`clear_context_compound_runs_rest_as_prompt`）此前已由
2026-08-11 的变更覆盖，但 preserved-plan 路径从未被测试触及，回归由此进入。

## 变更

### Session 层

- **`crates/session/src/runner/mod.rs`**（`run_with_registry` ClearContext 分支）：
  将无条件的 `user_text.clear(); images.clear();` 替换为 `match rest`——
  `Some(rest)` 时保留 trailing request（`user_text = rest`），仅 `None`（裸命令）
  时才清空。`handoff_pending = true` 保持不变，确保 plan handoff + request 一并
  交由 `run_loop` 执行。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 复合 clear_context 在保留计划分支下保留 request | clear_context_compound_keeps_rest_with_preserved_plan | tests/clear_context_regression.rs |

- 集成回归：`cargo test --test clear_context_regression` → 1 passed, 0 failed
- 相关回归：`cargo test --test control_cmd` → 15 passed, 0 failed
- session 库：`cargo test -p opencoder-session --lib` → 308 passed, 0 failed
- clippy：`cargo clippy -p opencoder-session --lib -D warnings` → 0 警告
- build：`cargo build --workspace` → Finished
- bug-catching 证实：去掉 fix 后测试失败（"trailing arg 'review' must be
  recorded as a real user prompt"），加上 fix 后通过。

## Impact Surface

- 用户：`/act_clear_context review` 在已有保留计划时，现在会清空上下文、保留计划，
  并将 `"review"` 记录为真实用户 prompt、随计划交接消息一并执行。
- 不影响：sentinel 路径（无 assistant 消息时行为不变）、bare 命令
  （`/act_clear_context` 无参数行为不变）、Store trait、LLM 后端、Web/CLI 结构。

## Related Docs

- [agents/session](../../agents/session/index.md)
