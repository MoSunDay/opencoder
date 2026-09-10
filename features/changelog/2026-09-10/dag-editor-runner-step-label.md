# DAG 编辑器「Runner 步骤」文案对齐

## 变更

DAG 编辑器两处显示文案 `Runner 工作流` → `Runner 步骤`：

- `crates/web/spa/src/dag/editor/canvasToolbar.jsx` 步骤面板标题
- `crates/web/spa/src/dag/editor/stepInspector.jsx` 类型下拉 label

旧文案暗示存在 workflow/sub-workflow 步骤类型，与并列的「Agent 步骤」「Wasm 步骤」命名不一致。wire 值 `'runner'`（PALETTE kindType / KIND_OPTIONS value / 线协议 tag）不变，spec 校验与执行路径零接触。

随同：

- 重建并提交 `crates/web/spa/dist`（`scripts/build-spa.sh`），否则 `html.rs` 编译期嵌入的 `app.js` 仍下发旧标签。
- `docs/registered-runners.md` 示例步骤名 `workflow` → `diagnose`（含 artifact 接口示例 `step=diagnose`），根除同一误导源。

## 测试覆盖

| 功能 | 测试或证据 |
| --- | --- |
| SPA 全量回归 | `npm test`（vitest run）→ 71 文件 / 563 passed / 0 failed |
| dist 与源码零漂移 | `scripts/check-spa-drift.sh` → no drift (build 1/3) |
| 嵌入产物编译 | `cargo check -p opencoder-web` → 零错误 |

相关语义：[Web](../../../agents/web/index.md)、[注册业务 Runner](../../../docs/registered-runners.md)。
