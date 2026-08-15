Commit: 2a89df3e3c01cc1e928b8eb52c71d22105172ebb

# 通用持久化 TODO 工作流

## Context

长任务需要同时保留全局状态和单 TODO 的干净高注意力上下文，并在进程或外部环境中断后可靠恢复。该能力必须不限于 UI，也不能把注意力边界误解为工具调用次数限制。

## Change Summary

- 新增 `opencoder-todos`：父 Workflow Primary Session 调度和验收，每个 TODO 使用独立 Primary Session 完整执行。
- Store schema 升级到 v9，新增 workflow、TODO projection 和 append-only event，使用 generation 乐观并发控制。
- 新增 `opencoder todos validate/run/resume/show/events/list/interrupt`；文件投影仅由 `run/resume --debug` 开启。
- 支持依赖 DAG、父决定并发批次、new/resume/fork、milestone 回退、硬工具门禁和持久化中断恢复。

## Impact Surface

- `crates/todos`
- `crates/store`
- `crates/cli` 与根 binary 分发
- builtin `workflow` agent

## Tests

| 合同 | 验证 |
|---|---|
| 父/子 Session 闭环与任务类型 | `parent_drives_focused_primary_todo_to_completion` |
| 父决定多 TODO 批次 | `parent_can_dispatch_multiple_independent_todos_in_one_batch` |
| debug 默认不落文件 | `normal_execution_does_not_create_a_debug_projection` |
| DAG 校验 | `dependency_validation_rejects_cycles_and_runnable_is_dependency_aware` |
| App crash/挂起后当前位置清理并恢复 runnable | `suspended_active_todo_becomes_recoverable_and_runnable` |
| Store 原子投影、事件、并发冲突和迁移 | `crates/store/tests/todos_workflow.rs` |
| CLI 解析和 debug 作用域 | `crates/cli/tests/todos_cli_parse.rs` |

全量门禁：`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`cargo build --workspace`。

## Related Docs

- [能力](../../todos/index.md)
- [逻辑](../../../agents/todos/index.md)
