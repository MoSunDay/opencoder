Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# Project overview/runs 对节点 404 execution-not-found 静默降级

节点 journal 丢失或重建（维护切换）后，执行索引仍指向该节点：overview 的 Inspect 与 runs 的 ProjectRuns 会拿到 `404 "execution not found"`。此前一律按失败渲染（行级 `detail_error` / runs 透传 404），把与 `Ok(None)`（无索引）等价的良性状态当成故障。现在与 `start` 里 pending resubmit 的恢复语义对齐：只有真实失败保持响亮。

## 变更

- `crates/control/src/api/project.rs` `overview`：Inspect 返回 `404` 且 body error 恰为 `"execution not found"` 时不写 `detail_error`，持久索引 `value["execution"]` 保持全量真相（如 interrupted）；节点离线 503 等其余失败仍写 `detail_error`。
- 同文件 `runs`：ProjectRuns 返回同一 404 时降级为空页 `{"runs":[],"next_version":null,"more":false}`，与 `Ok(None)` 无索引同形；其余回复原样透传。
- `crates/control/tests/e2e/support/node.rs`：`NodeOperation::ProjectRuns` 回退消息由 `"project execution not found"` 改为 `"execution not found"`，与真实 worker `validate_reference` 的按 kind 无关文案一致；`MockNode` 新增 `set_inspect_reply`（原始 `RpcReply` 注入，用于非 200 降级测试）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| overview 节点丢执行静默降级 | `overview_degrades_quietly_when_the_node_lost_the_execution` | `crates/control/tests/e2e/project_api.rs` |
| overview 真实失败保留 detail_error（503） | `overview_keeps_detail_error_for_real_inspect_failures` | `crates/control/tests/e2e/project_api.rs` |
| runs 节点丢执行降级空页 | `todo_runs_degrade_to_an_empty_page_when_the_node_lost_the_run` | `crates/control/tests/e2e/project_api.rs` |

- 定向回归：`cargo test -p opencoder-control --test e2e project_api` → 14 passed / 0 failed
- 全量回归：`cargo test -p opencoder-control --test e2e` → 173 passed / 0 failed

## 上线记录

- 2026-09-17：修复已随 `rel-9e66930a` 上线本机 Server（127.0.0.1:3048；与 Operator 更名、导航选择持久化同树发布，追发记录见同目录 `spa-nav-selection-persistence.md` 与 `2026-09-16/release-2868ebfd-signal-deploy.md` 同款 signal 部署链路）。本机验证：8 条受影响 `project-pt-*` todo 的 overview 均无 `detail_error`（修复前全部命中），`GET /api/project/todos/:id/runs` 返回 200 空页，内嵌 SPA 含 `menu:"Operator"`、无「会话交互」残留。
- 备注：曾为发布构建过 bundle `rel-5102151`（基于 Operator 更名提交）；因并行会话同树先发 `rel-9e66930a`（已包含本修复）而未部署，bundle 与临时 worktree 已清理。
