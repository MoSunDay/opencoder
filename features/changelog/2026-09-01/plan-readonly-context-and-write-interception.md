Commit: 8709349 (working-tree, plan 只读上下文与全写拦截)

# plan 只读上下文与全写拦截

## 背景

plan 的 Environment 标记曾允许 `/tmp` 写入，拦截文案也只描述“只读”而没有要求
模型停止实现尝试。结果是模型可能把写目标换到 `/tmp` 继续执行，或收到工具错误后
反复寻找另一条写路径；这与“plan 只做分析并输出计划”的真实契约不一致。

## 变更摘要

- Environment 仅在 plan 注入
  `MODE: plan (read-only); IN_PLAN_MODE=true`；act 不再携带任何 mode 描述行。
- plan system suffix 与所有 canonical 拦截信息统一要求：操作未执行、不要重试或寻找
  其它写路径、继续只读分析并只输出 plan。
- shellguard 的 `AllowReason`/`Verdict` 增加可组合的 `writes_state` 类型化效应。
  sandbox 层仍可放行落在 `/tmp` 的变更，但 session 的严格 plan adapter 会继续拒绝，
  避免把 sandbox release 错当成只读。
- edit/MCP 等未准入工具、build 子 agent、普通路径与 `/tmp` 下的 bash 写入都在执行前
  返回 `ToolOutput::err`；该 Tool 结果进入下一轮模型 context。精确 `/dev/null` 与 fd
  重定向不持久化状态，仍可用于丢弃/合并只读命令输出。

## 验证覆盖

- `environment_block_marks_plan_mode_readonly` / `environment_block_omits_plan_marker_in_act`
  固定 plan-only mode 行与 act 省略行为。
- `plan_mode_blocks_write_command` 同时验证未执行、model-facing ToolEnd，以及下一轮
  LLM request 已包含只读拦截、禁止重试和只输出 plan 的指令。
- `plan_mode_blocks_relative_write_in_tmp_call_workdir` 固定 `/tmp` 不再成为 plan 写旁路。
- session bash_guard compatibility corpus 覆盖 redirect、rm/mv/cp、shell recursion、find
  输出与 delete；shellguard 单测固定复合命令组合时写效应不丢失。
- `plan_mode_blocks_build_subagent` 固定 build 子 agent 使用同一 canonical 拦截信息且不启动。

- 全量回归：`cargo test --workspace` → 3786 passed / 0 failed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- build：`cargo build --workspace` 与 `cargo build --release --bin opencoder` → 成功。

## 本地部署

- release 产物版本为 `opencoder 0.1.0 (8709349-dirty)`，已原子替换 PATH 首选项
  `/root/.local/bin/opencoder`；部署产物与 release 构建的 SHA-256 均为
  `7d98872cc8a55c6183c19cb8b8a027dba23b1d34050ff03b166ee5a5401dd17e`。
- 上一版保留在
  `/root/.local/bin/opencoder.backup-before-plan-readonly-20260901`（SHA-256
  `8aefc521771f8ea874557a1052653706163836dfcf964908dc31bd867543b185`）。
- 部署时已在运行的 opencoder 进程仍持有旧映像；重新启动相应会话后加载新版本。

## Related Docs

- [agents/session](../../../agents/session/index.md)
- [agents/core](../../../agents/core/index.md)
- [agents/shellguard](../../../agents/shellguard/index.md)
