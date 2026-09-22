Commit: 3b4775905c950f64433b5c9f4439f4396674b3c6

# Brain step/能力分层调度

## 变化

计划统一为 schema 4 的 step 与连线：step 用一句话描述任务并关联一个能力，保存的固定版本计划也可作为能力。保留事件驱动和完整层屏障，同层并行、重试耗尽失败。Server 负责能力准入与索引，Worker 持有调度投影及模型激活；页面按执行索引复用现有运行明细。

删除旧调度器及对应 API、CLI、UI、测试和旧表创建逻辑；旧数据无自动迁移，清理需审阅精确范围并保留备份，包含 Host 休眠库存引用。运行卡片固定尺寸为重试标记预留空间，避免标题被挤压。

生产清理按审核清单摘要及主键校验，离线运行库先备份，共享 Host 表短事务处理；休眠索引裁剪持有 `runtime-use` 排他锁。失败恢复离线库与受影响的 Host 行/库存，重复执行验证已清除状态。未增加数据库版本、环境变量或修改鉴权数据。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 拓扑分层 | `kahn_levels_follow_edges_not_json_order` | `crates/brain/tests/layered/validate.rs` |
| 完整并行层屏障 | `parallel_nodes_in_layer_then_next_layer` | `crates/brain/tests/layered/barriers.rs` |
| 重试耗尽、迟到回执 | `retry_schedules_next_attempt_then_fails_run`、`late_terminal_from_previous_attempt_ignored` | `crates/brain/tests/layered/barriers.rs` |
| 嵌套真实执行 | `nested_plan_dispatches_a_real_child_and_reports_its_terminal_to_parent` | `crates/worker/tests/brain_nested.rs` |
| Server/Worker 端到端画布 | `layered_canvas_holds_the_barrier_then_completes_through_the_closing_activation` | `tests/brain_layered_e2e/canvas.rs` |

SPA 112 文件、886 项测试通过，卡片调整后相关 18 项复测通过；Clippy 全 workspace/all-targets 零警告；发布脚本 43 项、维护脚本 5 项测试通过。先执行 `cargo build --workspace --bins` 校验四个进程版本，再执行 `cargo test --workspace`：5505 passed / 0 failed / 7 ignored（已有默认忽略，浏览器验收独立执行通过）。真实浏览器覆盖创建、草稿、连线、执行、详情、嵌套、失败及空状态；生产真实模型验证并行屏障、固定版本子计划和两次失败上限。最终发布版 15 分钟稳定观察通过。

## 相关

- [能力与业务规则](../../brain/index.md)
- [Brain 逻辑](../../../agents/brain/index.md)
- [运行协议](../../../docs/brain-orchestration.md)
