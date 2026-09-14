Commit: 104c6b2663858f0a15d7066eb89227d681a4cf44

# TODO 模板新建/编辑改为全宽右侧抽屉

## Context

TODO 管理页的新建模板是页面内联表单、编辑模板会整页替换列表（TodoEditor 自带 Card 外壳）。用户明确要求与 Agent 详情一致的模式：右侧滑出抽屉、不要卡片叠卡片；且 TODO 模板编辑要占满 100% 宽（Agent 是 75%）。

## Change Summary

`todoPanel.jsx` 模板 tab：新建与编辑都改为 `<Drawer placement="right" size="100%">`（wrapper maxWidth 100vw，destroyOnHidden），列表保持在抽屉背后挂载；编辑不再整页替换，`closeEditor` 关闭即 bump 刷新列表（抽屉里的保存在列表视角立即可见）。`CreateTemplateForm` 去掉外层 Card（抽屉标题接管）。`todoEditor.jsx` 去掉 Card 外壳，改为 `.todo-editor` + `.todo-editor-toolbar` 布局：模式切换（表单/画布/JSON）在左，返回/保存在右，标题由抽屉提供。

## Impact Surface

- `crates/web/spa/src/todoPanel.jsx` — 两个 100% 抽屉（新建/编辑）、CreateTemplateForm 去 Card、移除编辑整页替换分支
- `crates/web/spa/src/todoEditor.jsx` — 去 Card 外壳，`.todo-editor` 工具条布局
- `crates/web/spa/src/app.css` — `.todo-editor-toolbar`（沿用画布任务的 `.oc-todo-run-*` 之后追加）
- `crates/web/spa/dist/*` — build-spa.sh 重建产物

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 抽屉契约（右侧、100% 宽、dialog role）+ 编辑器无外壳 + 列表仍在背后 + 返回关抽屉并刷新列表 | `opens version editing in a full-width right drawer with the chrome-less editor` | `crates/web/spa/src/todoPanel.dom.test.jsx` |
| 新建抽屉 + 示例 spec 原样上行 | `creates a template through POST /api/todo/templates` | `crates/web/spa/src/todoPanel.dom.test.jsx` |

`todoPanel.dom.test.jsx` 全文件 6 tests ≈ 3s。

## 排障记录（jsdom + RTL 性能坑）

编辑抽屉打开后，测试里 `screen.getAllByRole('button')` 会让单条 fireEvent.click 阻塞 45s+：RTL `queryAllByRole` 对每个候选元素跑 `isInaccessible` → jsdom `getComputedStyle`，而 antd CSSINJS 注入数千条规则后每次 computed style 解析都极贵（CPU profile 证实 45s 里的 38.5s 在 jsdom 样式解析，渲染只有 27 次）。规避：抽屉场景下用 `drawer.querySelectorAll('button')` + 文本归一化替代全局 role 查询（与 antd 两字中文按钮自动插空格的 `findButton` 约定并存）。新 DOM 测试在「全页抽屉 + 大样式表」场景应优先局部查询。

## 回归基线

- `npm test`（spa）681 tests：本次改动范围全绿；仅存 2 个失败位于他人未提交的 brain/workbench WIP（`plans.dom.test.jsx` 存储失败用例隔离复现、`editor.dom.test.jsx` 全量并发下偶发超时），与本改动无关。
- `scripts/build-spa.sh` 通过（输出契约校验 + dist 更新）；`check-spa-drift.sh` 因上述 WIP 中 `brain/workbench/editor/model.js` 的越界相对导入（`../../../../../examples/...`）在临时树构建失败，改用等价手段（临时树补 examples 符号链接后构建）确认 dist ↔ src 无漂移。
