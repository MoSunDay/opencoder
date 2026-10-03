Commit: a7e6b8233d5a4f6d0727d656d5d3d45a985b4b63

# TODO 初始化期间等待工作流建立

执行明细只有在当前执行已返回工作流时才加载 TODO 工作台。初始化、停止和初始化失败期间不会请求尚不存在的 Review 数据；初始化错误统一由状态提示展示一次。工作流建立后正常加载工作台，实际读取失败仍显示错误。

## 测试覆盖

| 行为 | 测试入口 |
| --- | --- |
| 初始化、停止中、已停止与失败状态不读取工作台，失败只显示一次 | `crates/web/spa/src/fleet/detail/embeds.dom.test.jsx` 的参数化初始化测试 |
| 初始化完成后打开工作台，保留读取失败的实际错误 | 同文件的初始化完成测试 |
| 浏览器中的初始化、停止、失败与恢复 | `scripts/acceptance/todo_initialization_ui.js` |

- SPA 全量 971 项、Rust 全量 5674 项通过；类型检查、格式检查、全目标 Clippy 和产物一致性检查通过。
- 四种屏宽与全站十五项 UI 验收通过。

[Web](../../../agents/web/index.md) · [TODO 工作台](../../todos/index.md)
