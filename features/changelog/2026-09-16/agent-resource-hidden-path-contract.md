Commit: 01c14802

# Agent 资源写侧拒绝隐藏路径段，对齐 memory 读侧契约（评审收口）

上一迭代（agent 全局激活下线 / @-提及 / memory 目录化）评审报告剩余风险 1、2 的跟进收口：修复「隐藏 `.md` 可写成功但注入静默消失」的写读不对称缝隙，并为 memory 聚合降级补排障日志。

## 变更

- 写侧拒绝隐藏段（web）：`crates/web/src/api_agent_resources.rs` 的 `safe_rel_path` 新增点前缀段检查，含 `.`/`..` 之外的隐藏段（如 `.hidden.md`、`dir/.x.md`）一律 400，错误信息携带具体段名；文档注释声明拒绝动机（读侧跳过点文件，可写不可注入属契约缺口）。上传、删除（`DELETE` 路径同走 `safe_rel_path`）全量生效。
- 写侧对称收口（agents 节点面）：`crates/agents/src/resources/model.rs` 的 `validate_path` 将 `part == "." || part == ".."` 收敛为 `part.starts_with('.')`，与 web 门同语义；错误仍为 `unsafe file path`。
- 读侧降级可观测：`crates/core/src/agent/memory.rs` 的 `section_body` 对不可读/非 UTF-8 文件由静默 `filter_map` 跳过改为 `tracing::debug!`（path + error）后跳过，降级语义不变（pool 永不失败整个 agent）。
- 未触碰面：SPA 与 dist 零改动（本次无前端行为变化）；`PATCH /api/agents/active` 等 T1 语义不变；1.5MiB 整包上限语义不变。

## 契约语义（修复后）

- 写入口径 = 读入口径：凡 `safe_rel_path`/`validate_path` 放行的路径，读侧聚合必然收集；读侧仍保留「跳过隐藏文件/子树」作为 staging 残留兜底（写侧已无法制造该状态）。
- 不可读/非 UTF-8 的 `*.md`：聚合时跳过并输出 debug 日志（此前无日志，排障时 memory「少一段」不可定位）。

## 测试清单（功能 → 测试名）

| 行为 | 测试名 | 位置 |
|---|---|---|
| 写侧拒绝隐藏文件/隐藏目录段 | `rejects_bad_category_paths_shape_and_oversize`（`.hidden.md`、`dir/.x.md` 并入 400 集合） | `crates/web/tests/web_agent_resources.rs` |
| 节点写侧对称拒绝 | `merge_preserves_bytes_modes_and_rejects_ambiguous_changes`（`.x`、`x/.y` 并入断言集） | `crates/agents/src/resources/model.rs` |
| 聚合降级跳过 + 空池无段 | `section_body_skips_unreadable_files_and_degrades`（新增） | `crates/core/src/agent/memory.rs` |
| 存量行为回归（聚合/排序/截断/单文件兼容） | `memory_multi_file_aggregation_orders_subtree_files` 等 | `crates/core/src/agent/tests/file_agents.rs` |

## 门与取证

- 全量回归：`cargo test --workspace --no-fail-fast -- --test-threads=4` → 406 个测试目标全部 `test result: ok`，5224 passed / 0 failed（较上轮 +1 为新增 core 降级用例），日志 `/tmp/followup-gate-20260916.log`。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 定向：`opencoder-core --lib memory::` 3/3、`opencoder-agents --lib resources` 1/1、`opencoder-web --test web_agent_resources` 7/7。
- SPA：无改动，未重跑（dist 逐字节不受影响）。
