Commit: (working-tree)

# Agent 列表一次性线上数据清理（无代码改动）

## 背景

- 「Agent 配置 → Agent 列表」数据源为 `GET /api/agents`（`crates/web/src/api_agents.rs`）：builtin 7 张 ∪ agents root（`/var/lib/opencoder-server/.opencoder/agents`，经 NFS 只读导出为 `/mnt/opencoder-agents`，同视图已核实）的 file 卡。清理前列表 67 项 = builtin 7 + file 卡 60。
- file 卡中仅 13 张有存活引用（5 条 DAG 定义的 agent 步骤引用恰好覆盖这 13 张）；其余 47 张全部无存活引用，属历史评测/适配残留。

## 预检（只读，全部通过）

- `GET /api/dag/defs` 共 5 条定义，agent 引用合计 13 个，与删除名单零交集：`eval-diagnose`→`eval-diagnose`；`regression-test`→`regression-test`；`static-auto-test-pingce`→`pingce-6dd5896907b5-*`；`static-auto-test-pingce-0bdf3c65d217`→`pingce-0bdf3c65d217-*`；`viking-dependency-analysis`→`viking-dependency-analysis-agent`。
- `pingce-f492332d16e6-*` 删除闸门：`pingce-formal-empty-20260916` 相关 run 均已终态。control.db `execution_index` 中引用 f492332d16e6 / formal-empty 的 3 条 dag run（`dag-pingce-0517…`、`dag-pingce-229b…`、`dag-pingce-b25e…`）状态均为 `done`（实为 `static-auto-test-pingce` 保留组定义的 dispatch 载荷内提及该标签）；当前唯一 running 的 pingce run（`dag-pingce-1730…`，`static-auto-test-pingce`）引用保留组 `pingce-6dd5896907b5-*`，不阻塞删除。
- `stat` 确认两个 agents 路径同视图（65 项列表一致：62 个 agent 目录 + `prompts`/`skills`/`tools` 共享池）。
- brain 绑定 active=null（全部指向 builtin `act`），不受 file 卡删除影响。

## 保留（builtin 7 + file 卡 13）

- builtin：`act`、`plan`、`explore`、`build`、`sidecar`、`command`、`workflow`（`act`/`sidecar` 同名卡受 builtin 守卫保留，列表合并为单行）。
- `pingce-0bdf3c65d217-{prepare,business,engineering,verify,finalize}`、`pingce-6dd5896907b5-{prepare,business,engineering,verify,finalize}`、`viking-dependency-analysis-agent`、`regression-test`、`eval-diagnose`。

## 删除（47 张，全部 `DELETE /api/agents/{name}` 返回 200）

- 9 组过期评测卡 ×5（45 张）：`pingce-{2a6085b45e07, 3f7cb3e26900, 3ffa2a881a97, 5f7d596cb966, 69c28c41b386, 88274fd6266e, aa36dfd32ec6, fe90a149a0ad, f492332d16e6}-*`。
- 2 张单卡：`viking-dependency-analysis-candidate`、`wrap-release-20260909`。
- 共享池（`prompts/`、`skills/`、`tools/`）不在本次范围，未触碰（其中孤儿池 `pingce-<hash>` 残留见「可选后续」）。

## 备份与验证

- 备份：`/var/lib/opencoder-platform/backups/agents-cleanup-20260916-151231/`（`agents-root.tar.gz` 1633 条目 + `agents-list-before.json` + `dag-defs-before.json`），沿用 `test-catalog-cleanup-20260916-091939` 的目录惯例。
- 删除后复核 `GET /api/agents`：共 20 项 = builtin 7 + file 卡 13，与保留名单逐一吻合，removed=47。
- 5 条存活 DAG 定义的 agent 引用全部仍可解析（missing=NONE）；保留卡的 `prompt_files`/`skills`/`tools` 共享池引用完好可解析；agents root 由 65 项降至 18 项（15 个 agent 目录 + 3 个共享池），计数自洽。
- 运行中的 `dag-pingce-1730…`（保留组）不受影响。

## 说明

- 纯数据清理，不改代码、不发版；rules/01（强制测试）、rules/02（回归门禁）不触发。
- 删除接口行为已在代码核实：拒 builtin、404 幂等、不触碰共享池、无运行时自动重建路径（`crates/agents/src/write.rs`、`crates/session/src/resume.rs`）。
- 可选后续（不在本次范围）：清理共享池孤儿（`agents/prompts/pingce-<hash>` 等 8 组已删卡对应池），可用 `DELETE /api/agents/resources/{cat}/{name}`（自带 409 引用守卫）。

## Related Docs

- [agents 模块](../../../agents/agents/index.md)

## 勘误（同日 15:23 追记）：列表会随评测持续再生长

- 本次删除的 47 张卡已确认无一复活（与当前 agents root 交集为 0）。
- 但 `static-auto-test-pingce` 评测流水线每次 run 都会新建一组 `pingce-<hash>-{prepare,business,engineering,verify,finalize}` 共 5 张卡，并把定义体重新指向新组，旧组随之失引用滞留 root——本次清理的 9 组孤儿正是该机制的历史积压。
- 实测复现：清理完成（15:12）后 3 分钟内，定义从 `pingce-6dd5896907b5-*` 改指新组 `pingce-eea771f5fb93-*`（15:15:52 落盘），并启动运行 `dag-pingce-57866e70…`（15:16:23，running）；旧组 6dd 即刻沦为孤儿。当前列表 25 项 = builtin 7 + 评测卡 18。
- 结论：**一次性删除无法保持列表干净**，根因是评测流水线向共享 agents root 泄漏卡片。持久修复需代码层（每次 run 终态后 GC 失引用组，或评测卡写入列表不可见的独立命名空间），属功能变更，触发 rules/01。

## 第二阶段（同日 15:57）：按用户决策移除 pingce 评测机制

用户决策：列表仅保留 `eval-diagnose`、`regression-test`、`viking-dependency-analysis-agent`（builtin 7 张为代码内置不可删）。

- 取消在跑 run：`dag-pingce-dcfdab9b…`（cancelling；其引用的组从未落盘，本已卡死）、`dag-pingce-57866e70…`（cancelled）。
- 删除定义：`static-auto-test-pingce`、`static-auto-test-pingce-0bdf3c65d217`（`DELETE /api/dag/defs/:id`，均 200）。至此「每次 run 新建一组卡并改写定义」的再生长源头移除。
- 删除卡片：`pingce-{0bdf3c65d217, 6dd5896907b5, e542e86c08c3, eea771f5fb93}-*` 共 20 张，全部 200（`/tmp/pingce_delete_results.txt`）。
- 备份：`/var/lib/opencoder-platform/backups/agents-cleanup-phase2-20260916-155255/`（root tar + 删前列表/defs + control.db 快照）。
- 终态复核：列表 10 项 = builtin 7 + 保留 3；剩余 3 条 defs 引用 missing=NONE；观察 3 分钟无重建（pingce 残留 0，无新 run）。
- 备注：提交源在平台外（本机无 cron/timer）；若外部 CI 再次提交 `static-auto-test-pingce`，定义与卡会被重建，届时需在提交侧停用。共享池孤儿（`prompts/skills/tools` 下 `pingce-<hash>` 组）仍在，可用 `DELETE /api/agents/resources/{cat}/{name}` 清理（自带 409 守卫）。
