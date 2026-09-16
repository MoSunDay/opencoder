Commit: df42d352eedc014416b8a37c743e49b6a68aeeb4

# 大脑发起表单：输入收敛为语义化描述 + 一层 KV 工程描述

大脑调度发起表单移除「高级选项」折叠区、「文档名称」和「input 名称」三个字段，Markdown 正文随具名文档概念一并退场。发起输入固定为两段：「目标和交付物」承载语义化描述；「工程描述（可选）」以 itemlist 形式组织一层 KV 对——键为计划声明的输入端口名，值支持 JSON 字面量（非法 JSON 回落原字符串，空键行忽略），用户不再需要理解端口命名与文档封装。

`launchBody` 的 `inputs` 直接由工程描述 KV 行构造（`engineeringInputs`），后端 `/api/brain/runs` 契约（`inputs: {端口名: 值}`）不变；原具名文档改为通过 KV 行以 JSON 值提交（如键 `document`、值 `{"name":...,"markdown":...}`）。运行中 `waiting_input` 按需询问路径不受影响。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 工程描述 KV 行构造 inputs（JSON 解析/字符串回落/空键忽略） | `submits the engineering description as one-level KV inputs and keeps fixed and generated plans explicit` | `crates/web/spa/src/brain/workbench/tests/model.test.js` |
| 移除字段不可见、默认零行一句话发起、KV 行进入请求 | `collects engineering inputs as a one-level KV list and launches a one-liner without them` | `crates/web/spa/src/brain/workbench/tests/launch.dom.test.jsx` |
| durable 提交语义（回归） | `keeps a durable run identity across an uncertain submission and exposes errors` | 同上 |
| 浏览器验收：经工程描述 KV 提交具名文档并完成执行 | `edit_publish_and_run_graph_in_browser`（脚本内改用 添加工程参数 填 KV） | `scripts/acceptance/brain/runtime.js` |

- SPA 全量回归：`npm test`（vitest）→ 109 文件 / 799 项全部通过；`npm run build` 成功，`crates/web/spa/dist/static/app.js` 随构建更新
- 后端 Rust 契约无改动，`/api/brain/runs` 请求体结构不变

## Related Docs

- [features/brain](../../brain/index.md)（发起输入描述已同步）
- [运行协议](../../../docs/brain-orchestration.md)（`inputs` KV 契约不变）
