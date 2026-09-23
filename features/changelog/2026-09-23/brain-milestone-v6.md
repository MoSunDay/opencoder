# 大脑里程碑计划与自由回退

计划画布按层配置里程碑及挂载能力；编辑节点后，通过计划信息表单保存版本。运行时大脑在整层执行结束后判断下一层，可回退到已执行层开启新轮，并为重跑能力生成独立执行 ID，保留历史输入与结果。无效决策最多纠正两次，不派发无效能力。运行视图按轮、层和能力 ID 打开原有类型执行面板。

窄屏画布在视口变化后重新适配节点位置，工具栏按钮保持可见，React Flow 控件限制在画布内。检查器说明与无需预设回退连线的规则一致。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 无效决策纠正与禁止派发 | `invalid_decision_is_corrected_before_any_capability_is_dispatched`、`three_invalid_decisions_block_without_dispatching_a_capability` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 回退后独立激活与历史记录 | `failure_wakes_the_brain_and_reflection_creates_a_distinct_durable_visit` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 画布编辑、提交与失败草稿 | `在真实画布编辑节点后，表单提交保存新版本且不会启动运行`、`提交失败后保留节点与表单草稿，返回画布可继续修改` | `crates/web/spa/src/brain/workbench/milestone/editor.dom.test.jsx` |
| 轮次历史和执行明细 | `历史轮保留各自输入、反思和执行 ID`、六种类型面板测试 | `crates/web/spa/src/brain/workbench/milestone/run.dom.test.jsx` |
| 窄屏布局 | 390px 与 1440px 浏览器截图、宽窄切换后节点可见且控件不越界 | `/tmp/brain-v6-canvas-narrow.png`、`/tmp/brain-v6-canvas-wide.png` |

- SPA 全量回归：`npx vitest run` → 906 passed / 0 failed。
- SPA 构建：`npm run build` → 通过；`scripts/check-spa-drift.sh` → 无漂移。
