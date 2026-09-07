Commit: cef0033

# Web 会话选节点、完成态 Markdown 与大脑能力库交互

## 行为变化

- 会话必须明确选择可用执行节点。创建、历史列表、模型与技能目录均指向所选节点；创建失败保留请求 ID 和草稿，切换节点后忽略旧列表和快照。读取事件水位失败时停止下发并保留草稿。
- 完成的 Agent 回答使用完整、安全的 Markdown DOM 渲染；标题、粗体、列表、代码、表格和链接保留结构，Say/Step 头部不重复正文首行。流式阶段继续显示原始预览。
- 大脑调度仅需目标节点和需求，提交后直接打开执行详情。空能力库由默认 act Agent 执行；有能力库时继续规划和选择能力，未绑定目标的能力默认 act。实际存储、规划和节点错误照常返回，重试沿用同一执行回执。
- 能力库列表与搜索结果共用表格。点击行编辑或点击新建均从右侧打开占视口 75% 的抽屉；编辑读取完整内容，保留工程输入，部分保存失败后重试复用已创建 ID。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| Markdown 完整结构、首行不重复与完成切换 | `completed Say Markdown and streaming preview` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| Node 必选、断线重试、旧响应隔离、水位错误与节点模型 | `explicit conversation node selection` | `crates/web/spa/src/chat/nodeSelection.dom.test.jsx` |
| 原会话发送、控制命令、队列与快照折叠 | 会话 DOM 回归 | `crates/web/spa/src/chat.dom.test.jsx` |
| 大脑选择节点后直接执行、修改需求/节点后的幂等键 | 大脑调度 DOM 回归 | `crates/web/spa/src/fleet/brain.dom.test.jsx` |
| 单表、右侧 75% 抽屉、新建/编辑、搜索工程输入与失败重试 | `capability library table and editor` | `crates/web/spa/src/brainPanel.dom.test.jsx` |
| 空库默认执行及回执重放、节点和计划错误、规划失败不可执行 | `empty_library_executes_on_selected_node_and_retries_the_same_receipt` 等四个 HTTP 用例 | `crates/control/tests/e2e/brain_api/default_execution.rs` |
| 既有能力 CRUD、绑定、幂等、准入和规划语义 | 拆分保留的 HTTP 回归 | `crates/control/tests/e2e/brain_api/` |

## 验证记录

开始基线为 `cef0033`（上一代码版本 `8ed37a1`）：Rust 4,752 passed / 0 failed / 5 个既有手动 ignored；SPA 416 passed。

本轮验证原始输出及浏览器、发布证据保存在 `/data00/opencoder-delivery/20260907-node-brain-interactions/`，最终结果在部署验收后补齐。Server 使用独立系统账号，模型及向量配置保存在 Git 外，沿用现有凭据；未修改或删除已有鉴权数据。

## 相关文档

- [Web 模块](../../../agents/web/index.md)
- [控制平面](../../../agents/control/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
