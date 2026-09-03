Commit: (working-tree, 基于 5410f6d)

# sidecar 获只读 bash；快照永不自动压缩

## Context

侧车问询只有 read/search/ls 三个工具：快照不够、需要 `git log`、`grep -r`、`wc -l`
这类仓库级检验时无路可走，回答质量受限于静态快照。同时 sidecar child 继承了父会话
的 compaction 配置：父会话逼近压缩阈值时，**每个** sidecar 问题都要先付一轮压缩
LLM，且摘要会替换掉借用来的快照——后续追问失去它们赖以回答的上下文。

## 根因

- 工具集在 `core::agent::builtin_agents` 里是 `Allow(["read","search","ls"])`，无 bash。
- `bash_guard::gate` 只识别 `AgentKind::Plan`；sidecar 是 `Subagent` kind，即使给了
  bash 也没有任何写效应拦截。
- `runner/sidecar.rs` 构建 child 时未关 `compaction.auto`。

## Change Summary

- `crates/core/src/agent.rs`：sidecar 工具集加入 `bash`；`base_prompt_sidecar` 声明
  "bash 仅只读检验命令（git log、grep、wc），每个写命令都会被拦截——不要重试或另找写路径"。
- `crates/session/src/bash_guard.rs`：`gate` 从 plan 专属扩展为只读会话门——`kind == Plan`
  或 `agent_name == "sidecar"`（bare kind 检查会漏掉 Subagent kind 的 sidecar）。双层
  fail-closed 不变：未准入工具拒（`SIDECAR_ADMITTED = read/search/ls/bash`）+ mutating
  bash 拒；新增 `sidecar_denial` 报 "Blocked in sidecar"，与 plan 拒文同样要求不换路径重试。
- `crates/session/src/runner/execute.rs`：gate 调用传入 agent name（cwd 对齐逻辑不变）。
- `crates/session/src/runner/sidecar.rs`：`child.config.compaction.auto = false`——
  transcript 是借用的父快照而非本回路持久历史，压缩 = 每问一轮额外 LLM + 摘要替换快照。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| sidecar 的 mutating bash 被拦且报 "Blocked in sidecar"；`ls`/`git log` 放行 | `session/src/bash_guard.rs::gate_blocks_mutating_bash_in_sidecar` |
| sidecar 未准入工具（edit/task/question）拒；read/search/ls 放行 | `session/src/bash_guard.rs::gate_refuses_unadmitted_tool_in_sidecar` |
| sidecar 工具集 pin 为 read/search/ls/bash | `core/src/agent.rs::sidecar_observer_is_read_only` |
| 超阈值父快照不触发压缩：轮数恰 1、无 Compaction/TranscriptReset 帧、快照逐字保留 | `session/tests/sidecar_loop.rs::sidecar_never_compacts_over_threshold_snapshot` |

## 回归

- 按用户指示本轮跳过 clippy/test（fmt 已过）；下一轮迭代前需补全量回归。
- plan 门行为不变（既有 gate 测试随签名更新，断言原样）。

## Related Docs

- [session 模块](../../../agents/session/index.md)、[core 模块](../../../agents/core/index.md)（sidecar 工具集与只读契约已同步）
