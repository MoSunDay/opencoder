Commit: 4eecc0f9d72d0643f73d7502ceb2c750e4c60982

# 专项与 TODO 详情全屏展示

专项详情去掉 1200px 宽度限制，TODO 详情去掉 1000px 外层宽度限制。两类详情沿用右侧抽屉，在各屏宽下覆盖整个视口；关闭 TODO 返回专项看板。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 项目进入专项，再进入 TODO，两层抽屉均全宽 | `opens project progress, then the initiative board and TODO in full-width right-side drawers` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 四种屏宽下两类抽屉的实际视口边界、关闭和返回 | `PASS fullscreen initiative and TODO drawers` | [project_workbench_ui.js](../../../scripts/acceptance/project_workbench_ui.js) |

SPA 全量 980 项、Rust 全量 5686 项通过；Chromium 在 1920、1280、768、390 四种宽度下验证两类抽屉，共 8 组边界测量通过。工作区构建、格式检查、Clippy、全站 UI、原生平滑升级与发布脚本回归通过。

版本 `4eecc0f9` 已通过 Server 信号发布。线上四种屏宽验证通过，连续 900 秒观察完成，154 次任务执行与 4501 次就绪采样无失败。

相关：[项目能力](../../project/index.md)、[Web 逻辑](../../../agents/web/index.md)。
