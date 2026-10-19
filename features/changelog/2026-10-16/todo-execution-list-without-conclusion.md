Commit: 02c66f83d42154b41b0dddfdea8dc5947ea5a555

# TODO 执行列表移除执行结论

TODO 详情的“执行”页签只保留能力、名称、执行 ID、状态和操作等关联记录。执行结论、过程、消息和产物统一通过执行 ID 打开的原生执行详情查看，列表页不再单独读取或展示结论。

## 验证

| 功能 | 验证入口 |
| --- | --- |
| 执行列表不显示“执行结论”且不请求结果接口，关联记录和执行链接仍保留 | [`todosTab.dom.test.jsx`](../../../crates/web/spa/src/project/todosTab.dom.test.jsx) |
| SPA 生产构建 | `npm run build` |

相关说明：[项目工作台](../../project/index.md)、[项目模块约定](../../../rules/07-project-module-contract.md)。
