Commit: fc047704e4c583cb9e0c11293b3815916ce1659b

# DAG 调度与画布编辑器支持 max_concurrency 配置

DAG 调度此前固定最多同时执行 4 个步骤。本次在 spec 增加 `max_concurrency`（整跑并发上限，1..=30、默认 4），并让画布编辑器保留和编辑该配置，避免画布结构编辑丢失并发设置。

本次同时交付已验证的领域、调度、API 和画布改动：

- `DagSpec` 反序列化缺省仍为 4；领域校验拒绝范围外的值。运行时按冻结 spec 的上限共享调度静态步骤和动态实例，API 保存并读回配置。既有运行仍使用自己的冻结 spec。

- `specValidate.js` 新增镜像常量 `MAX_CONCURRENCY = 30`（对应服务端 `crates/dag/src/policies.rs` 的 `MAX_CONCURRENCY`，`default_concurrency` 为 4）；`validateSpec` 在 name 检查之前校验：字段出现时必须是 1..=30 的整数，否则报 `spec.max_concurrency 必须是 1..=30 的整数`。缺省仍合法（落库走服务端默认 4），服务端 400 问题列表照旧透出。
- `editor/canvasModel.js` 的 `canvasToSpec` 在 baseSpec 的 `max_concurrency` 为有限 number 时透传到重建结果，未设置则整体省略；画布结构编辑（加步/改名/连线/删步）不再丢并发配置。
- `editor/stepInspector.jsx` 的 `SpecMetaForm`（未选中步骤时的基础信息面板）在描述之后新增「并发上限（1-30，默认 4）」InputNumber（min 1 / max 30 / precision 0，placeholder「默认 4」），清空提交 `undefined` 即删除该字段。
- `defEditor.jsx` 的 EXAMPLE JSON 模板在 name 之后加 `"max_concurrency": 4,` 提升可发现性；不影响现有断言（`src/todo/directory` 的同名示例属于 todo 目录模块，互不相关）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 缺省及边界校验 | `max_concurrency_defaults_to_4`、`validate_rejects_out_of_range_max_concurrency` | `crates/dag/src/domain.rs` |
| 并发 2 的启动门禁及并发 1 串行执行 | `max_concurrency_gates_simultaneous_step_starts`、`max_concurrency_one_runs_strictly_serial` | `crates/dag-runtime/tests/run_loop/concurrency.rs` |
| API 保存读回及范围拒绝 | `def_upsert_roundtrips_max_concurrency_and_rejects_out_of_range` | `crates/web/tests/dag_api.rs` |
| 画布 roundtrip 保留 max_concurrency | `roundtrip 保留顶层 max_concurrency（画布结构编辑不丢并发配置）` | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 未设置 / undefined 时省略字段 | `roundtrip 省略未设置 / undefined 的 max_concurrency（落库走服务端默认 4）` | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 边界 1 / 30 合法、缺省合法 | `accepts max_concurrency bounds 1 and 30 (absent stays legal)` | `crates/web/spa/src/dag/specValidate.test.js` |
| 0 / 31 / 2.5 / '4' 报错 | `flags out-of-range / non-integer / non-number max_concurrency` | `crates/web/spa/src/dag/specValidate.test.js` |
| 画布加步骤保存不丢并发配置 | `结构编辑保留顶层 max_concurrency（加步骤后保存不丢并发配置）` | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 基础信息面板编辑并发并保存 | `基础信息面板可编辑并发上限并保存` | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |

- 本轮完整验证回执与最终数量统一记录于同日 `storage-admission-threshold.md`；包含工作区 Rust、SPA 和产物一致性检查。
