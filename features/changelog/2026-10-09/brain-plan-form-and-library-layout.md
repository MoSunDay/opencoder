Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# 计划统一表单与能力库表格整理

点击当前计划名称或“修改”直接打开新建计划使用的全屏画布与计划信息表单。保存修改仍生成同一计划的下一版本；历史查看、比较和固定版本执行通过“历史版本”入口保留，旧格式须显式转换。请求失败显示错误，迟到读取不会覆盖新建表单。

能力库移除输入描述、输出描述和工程输入列，这些内容仍在详情表单完整展示。一句话描述限制为 32rem，超出部分显示省略号；表格采用固定布局，窄屏在表格内横向滚动。列表和搜索结果使用相同展示与详情入口。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 同一表单新建、修改、草稿恢复、历史只读及读取错误 | `点击计划名称直接复用新建画布和表单，保存同一计划的新版本` 等 6 项 | [plans.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/tests/plans.dom.test.jsx) |
| 列表精简且详情保留完整输入、输出和工程示例 | `shows a single table and keeps all forms in the drawer`、`opens a clicked table row in edit mode and saves the full content through PUT` | [brainPanel.dom.test.jsx](../../../crates/web/spa/src/brainPanel.dom.test.jsx) |
| 四种屏宽下的长描述、搜索、能力详情、新建与修改表单 | `checkWidth` / `measure` | [layout.js](../../../scripts/acceptance/brain/ui/layout.js) |

- Brain 相关前端测试：9 个文件、68 项通过。
- 浏览器显示验收：1920、1280、768、390 四种屏宽，共 24 项通过；使用隔离的确定性接口数据，不等同于真实调度验收。
- SPA 构建、`cargo build --workspace`、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 通过。
- SPA 全量回归：1060 项通过、12 项失败，另有 1 个未处理错误；失败位于本次未修改的 `chat/*` 与 `sse.resume.test.js`。没有宣称全量通过。
- `cargo test --workspace` 未完成，进程退出码为 143；全站真实服务、配套原生镜像和 TUI 验收未执行。
- 发布脚本基础测试 19 项、信号测试 12 项通过；rolling 测试被终止，maintenance 测试超过 12 分钟后停止，均未计为通过。

当前能力和模块入口见 [Brain 功能](../../brain/index.md) 与 [web 模块](../../../agents/web/index.md)。
