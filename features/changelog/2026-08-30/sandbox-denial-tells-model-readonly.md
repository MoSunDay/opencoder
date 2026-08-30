Commit: (working-tree, sandbox 拦截模型可见拒绝——统一沙箱门 + 修正逃生口 /act + 禁止重试话术)

# sandbox 拦截必须「告诉模型你在沙箱、不可写、别再试」——非沙箱白名单工具门 + 拒绝话术收敛

## 问题与根因

用户观察：沙箱模式拦下写操作后模型仍反复尝试写。盘点 `execute` 派发前的拦截面，根因有两个：

1. **拦截面只盖 `bash`**：sandbox 会话的 schema 只广告 `bash/task/question`（agent allowlist），但执行注册表是全集——模型凭残存记忆/幻觉调 `edit`、`bg` 或未广告的 MCP 工具时**直接执行、静默写盘**，连「你在沙箱」的反馈都没有；这是洞而不只是话术问题。
2. **拒绝话术指错逃生口**：bash 写拦截文案写的是 `(/agent act)`——该命令不存在（真实命令是 `/act`），模型按图索骥失败后继续盲试写，直到 doom-loop（20 次同签名）或 tool-failure guard（20 次连败）才被强杀。

## 变更

### 行为（`crates/session/src/bash_guard.rs` + `runner/execute.rs`）
- **统一沙箱门 `bash_guard::gate(kind, tool, command)`**（纯函数，execute 内联块收敛为 8 行调用，execute.rs 795→789 行）：
  - 非 Sandbox kind 直接放行（act/explore/build/workflow 行为零变化）；
  - Sandbox kind 下工具不在 `SANDBOX_ADMITTED`（= sandbox ToolFilter 白名单 `bash/task/question`，带一致性单测防漂移）→ 拒绝，工具体**永不执行**；`question` 仍由下游 latent 门裁决，`task`→`build` 仍由 subagent 门拒绝（「不告诉模型 build 存在」语义保持）；
  - `bash` 仍过 shellguard 分类器，变异命令拒绝。
- **拒绝话术 `sandbox_denial(tool, detail)`**（重试抑制契约）：点名沙箱模式 → 声明 read-only 不可写 → 明说 "Do not retry: every write attempt fails while sandbox mode is active" → 逃生口改为真实命令 `` `/act` ``（绝不出现 `/agent act`）。bash 拒绝携带 shellguard reason 作为 detail。
- **非目标**：释放集（`/tmp` + `/dev/null`，cwd 不释放）、shellguard 分类策略、`/act`·`/sandbox`·`/act_clear_context` 语义、act 模式不设防，均不变。

### 文档同步
- `tests/bash_guard_sandbox_mode.rs` 模块 doc 重述三契约（bash 变异拒绝且禁止重试、未广告工具拒绝且不执行、只读放行）。

## 测试清单

| 类别 | 测试名 | 位置 |
|------|--------|------|
| 新增 | `denial_names_mode_forbids_retry_points_at_act`（含 `/agent act` 反向断言） | bash_guard.rs（lib tests） |
| 新增 | `admitted_set_matches_sandbox_agent_tool_filter`（白名单漂移钉） | bash_guard.rs |
| 新增 | `gate_passes_non_sandbox_kinds_through` / `gate_refuses_unadmitted_tool_in_sandbox` / `gate_blocks_mutating_bash_in_sandbox` | bash_guard.rs |
| 新增 | `sandbox_mode_refuses_unadvertised_tool_without_executing`（hallucinated `edit`：is_error + 沙箱拒绝 + **文件未被写**） | tests/bash_guard_sandbox_mode.rs |
| 修正 | `sandbox_mode_blocks_write_command` 断言 `` `/act` `` 存在、`/agent act` 不存在、"Do not retry" 存在 | tests/bash_guard_sandbox_mode.rs |

## 全量回归

- `cargo test --workspace --no-fail-fast -j 4`：**241 suites / 3729 passed / 0 failed**（当前树终账，REGRESS_EXIT=0；含本项 6 新增 + 1 修正测试，基线 3686 → +43，存量无回归）。
- clippy `-p opencoder-session --all-targets`：0 告警。
- 行数：`bash_guard.rs` 344、`runner/execute.rs` 789、`tests/bash_guard_sandbox_mode.rs` 372，均在限内。

## Impact Surface

- sandbox 模式下任何写路径（bash 变异 / 幻觉工具 / 未广告 MCP 工具）都收敛为**同一条模型可见拒绝**，模型第一时间知道「沙箱、只读、重试无用、切 /act」，不再烧满 20 次 doom-loop 才停。
- 不影响：act/command/workflow 会话、explore/build 子代理、shellguard 判定本体、释放集、latent/question 门、store/web 边界。

## Related Docs

- [agents/shellguard](../../agents/shellguard/index.md)
- [bash_guard 换壳 shellguard](../2026-08-30/bash-guard-shellguard-swap.md)
- [/act_clear_context sandbox 收敛 act](../2026-08-30/act-clear-context-sandbox-convergence.md)
