Commit: cbc3c311082c070a4169e408c1c20fb202084476

# 大脑运行画布与层执行详情

运行画布随可用区域伸展，支持全屏、响应式排列和键盘操作。层与执行节点均从完整操作记录定位实际执行；单任务直接打开，多任务和历史轮次可选择，未派发层显示明确说明。固定节点尺寸并保留测量结果，防止切换详情后画布空白。调度协议与数据结构不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 已完成层与最新执行选择 | `运行推进到下一层后，点击前一层和节点仍打开该层最新执行` | [run.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/milestone/run.dom.test.jsx) |
| 并行层与未派发层 | `并行层展示全部任务，未派发层显示明确说明`、`未派发层支持键盘打开且不伪造执行记录` | [run.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/milestone/run.dom.test.jsx) |
| 全屏与终态操作 | `全屏画布支持退出按钮和 Escape，完成后不展示无效的调度操作` | [run.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/milestone/run.dom.test.jsx) |
| 历史排序与创建中边界 | 全部纯函数用例 | [model.test.js](../../../crates/web/spa/src/brain/workbench/milestone/runtime/model.test.js) |
| 真实层详情与四档屏宽 | 四层执行读取、全屏、页面无溢出 | [run-canvas.js](../../../scripts/acceptance/brain/ui/run-canvas.js) |

- SPA 全量：160 个文件、1,136 项测试通过；里程碑专项 30 项通过。
- Rust 全量：复用同源 `cargo test --locked --workspace --no-fail-fast` 结果，5,747 passed / 0 failed / 8 个原有 ignored；1,755 个文件与测试源摘要相同，清单未覆盖的 45 个 Rust 文件与其基线提交无差异。原始日志与比对回执保留在 `/var/tmp/opencoder-brain-canvas-evidence/shared-regression/` 和 `shared-rust-source-comparison.json`。
- 格式检查、Clippy 零警告、workspace 构建及 66 项发布脚本测试通过。
- 本发布包全站 UI 16 项检查全部通过，覆盖 17 个页面、四档屏宽、原生执行及 TUI；回执 `/var/tmp/opencoder-brain-canvas-evidence/release-ui/receipt.json`。
- 发布 `rel-cbc3c311082c070a4169e408c1c20fb202084476`，线上四档屏宽各打开四层实际执行，页面摘要与发布包一致；15 分钟、180 次公共入口检查通过。回执 `/var/tmp/opencoder-brain-canvas-evidence/publication.json`。
- 首次线上浏览器验收因继承环境代理失败，保留原记录；清除验收进程代理后直连公开地址通过，未修改生产代理配置或认证数据。

## 相关索引

- [大脑调度工作台](../../brain/index.md)
- [Web 模块](../../../agents/web/index.md)
