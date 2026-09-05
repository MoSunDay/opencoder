Commit: 3d971f0

# Agent 激活 preflight 拒绝无 prompt 卡（修复静默失效）

## Context

[评审报告](../2026-09-04/opencoder-agent-versioned-agents.md)（问四 #6 / 问五 #1）指出：`PATCH /api/agents/active` 的 preflight 对 `current.prompt == None` 返回 `Ok(())`，但 `resolve_agent` 要求 prompt 引用存在才成卡——激活无 prompt 卡会成功落 marker，读路径却把该名解析为 None、`effective_default_agent` 静默回落 act，激活形同失败且无任何报错。低危（无数据损坏/安全问题），建议上线前修复。

## Change Summary

- **`crates/web/src/api_agents.rs::patch_active`**：preflight 的 `None` 分支改为 `Err("card \`{name}\` has no prompt reference — not a resolvable agent (reads would silently fall back to act)")`——经 `set_active_agent_checked` 映射为 InvalidData ⇒ HTTP 400 + marker 回滚，激活前即报错；doc/inline 注释同步。SPA 无需改动：`agentDetail.jsx` 的 catch 已按 `e.message` 透出 400 文案（注释原就预期「prompt 预检失败」）。
- **`crates/web/tests/web_agents.rs`**：新增 `seed_prompt_pool` 助手（写活 `prompts/<名>` 池：meta current=1 + v1/soul.md）；6 个原「激活无 prompt 卡却期望 200」的测试改为持活 prompt 引用卡（`cards_crud_activation_and_listing`/`repeat_activation_fans_reload_once`/`blank_active_name_is_400_not_deactivation`/`put_fans_reload_only_for_active_card`（hot 用 old-pack→pack 保持真变更）/`delete_active_card_clears_marker_and_fans_reload`/`patch_preflight_missing_prompt_rolls_back`）；新增 `patch_preflight_promptless_card_rejected_and_rolls_back`。
- **记忆同步**：`agents/web/index.md`（preflight 语义补「prompt 引用缺失即 400」）、`agents/agents/index.md`（resolve 契约句补 web 激活同据拒绝）。
- 范围外附带（披露）：并行流把 `crates/web/src/api_nodes_dag.rs` 留在编译失败中间态（`String` 传给 `error_409(&str)`），做了一行机械修复（`format!` → `&format!`）以解锁验证；该文件主体改动属并行流。

## 测试覆盖（规则 01）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 无 prompt 卡激活被拒（4xx + ok=false + 错误含 prompt + marker 回滚保持旧值） | `patch_preflight_promptless_card_rejected_and_rolls_back` | `crates/web/tests/web_agents.rs` |
| ghost prompt 引用激活 preflight 回滚（既有语义回归确认） | `patch_preflight_missing_prompt_rolls_back` | `crates/web/tests/web_agents.rs` |
| 激活/幂等/去激活 fan_out 恰一次（改为持活 prompt 卡后语义不变） | `repeat_activation_fans_reload_once`、`cards_crud_activation_and_listing` | `crates/web/tests/web_agents.rs` |
| 非生效卡改引用静默 / 生效卡 fan_out（含真变更 old-pack→pack） | `put_fans_reload_only_for_active_card` | `crates/web/tests/web_agents.rs` |
| 删生效卡清 marker + fan_out / 空白名 400 非去激活 | `delete_active_card_clears_marker_and_fans_reload`、`blank_active_name_is_400_not_deactivation` | `crates/web/tests/web_agents.rs` |
| 共享池写 reload 策略（激活即含活 prompt 卡，不受影响） | `reload_only_for_active_chain_writes` 等 5 套件 | `crates/web/tests/web_agent_resources.rs` |

## 回归（实际输出）

- `cargo build --workspace` → Finished，零错误。
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- `cargo test -p opencoder-web` → 11 个测试二进制全部 ok、0 failed。
- `cargo test --workspace --no-fail-fast` → 3952 passed / 0 failed（294 条 result-ok 行，含 opencoder-store——并行流 store 中间态已在其流内收敛为绿）；另 8 个 target（session --lib 与 7 个 web 非本特性套件）在 5 路并发 cargo 争用下无 result 行中止，逐一独立复跑全绿：web 7 target 22 passed / 0 failed、`opencoder-session --lib` 439 passed / 0 failed。
- 行数 gate：`api_agents.rs` 277 行、`web_agents.rs` 514 行（迭代中 ≤800）。
- **终态复验（当前树，含并行流后续合入）**：`cargo test -p opencoder-web` → 56 个测试二进制 280 passed / 0 failed；`cargo test --workspace --no-fail-fast` → 4418 passed / 0 failed（302 条 result 行，无 target 中止，无需复跑）；build/clippy（`-D warnings`）零错零警。
