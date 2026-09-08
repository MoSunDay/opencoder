Commit: (working-tree, 基于 002f9e07117c633cdb5764dad80cab5ade241776)

# 执行详情宽度与会话角色头像

执行详情统一从屏幕右侧向左滑入，宽度为视口的 75%，在不同入口和屏幕尺寸下保持同一比例。Agent 回答使用机器人头像，用户输入使用用户头像；移除回答中的 Say 文字标识，步骤数量、流式预览、Markdown 正文和逐层展开交互继续保留。

共用渲染器使全部执行、Agent 启动结果和会话交互保持一致。前端静态构建产物随源码更新。

## 测试覆盖

| 功能 | 测试名 / 验证 | 文件 |
| --- | --- | --- |
| 机器人与用户头像、步骤/工具展开及刷新保留状态 | `pairs separate persisted messages and retains nested disclosure on refresh` | `crates/web/spa/src/fleet/detail/transcript.dom.test.jsx` |
| 步骤数量、流式预览、完成态 Markdown 正文 | `StepsContent three-level drill-down`、`completed Say Markdown and streaming preview` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| 右侧滑入、75% 宽度、头像、工具展开、Escape 与遮罩关闭 | Chromium 加载最新 dist，使用本地消息与 API 夹具 | 临时浏览器验收 |

- SPA：`npm test` → 53 个文件、468 个测试通过；`npm run build` 通过（保留已有单包体积提示）。
- 浏览器：1600 / 1024 / 390px 视口中，抽屉实测宽度分别为 1200 / 768 / 292.5px；角色头像和展开/关闭操作通过，无页面运行错误。
- `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo build --workspace` 通过。
- Rust 全量回归取得 4814 passed / 0 failed / 5 ignored；后续取消收尾修复另通过项目模块 28 项单元测试与 14 项集成测试。测试使用满足节点存储门槛的 `/var/tmp`，回环请求排除代理。

## 发布验证

- 发布代码：`ff43bfa9410769695481374ea3bd2c5d7e08867c`，已同步 GitHub main；2026-09-08 18:34（北京时间）完成 Server、Agent、CLI 同代切换。
- 先冻结调度、收敛执行并制作一致性备份，再安装同一 manifest 的三二进制。节点 ID 保持一致，线上静态资源与包内摘要匹配，既有执行索引完整。
- Chromium 在线实测 1600 / 1024 / 390px 三种视口，均为 75% 右侧抽屉；机器人和用户头像各自显示，无 Say 标识、无页面运行错误。
- 真实模型验收通过 Agent、DAG 产物下载、团队、项目 Plan/Act 与回放、大脑稳定 request_id 重试、interrupt 和五字段索引检查。
- 最初 Agent 创建确认超时后按相同 ID 确认已完成；团队提示词与协调 JSON 冲突后使用明确的阶段格式重新验收。过宽的项目验收提示触发开发规划，已中断该测试执行，改以口算任务验证完整链路。
- 候选包完成 34 次项目运行与 30 分钟稳定性观察；最终代码的 34 次运行已通过功能断言，持续观察及生产两小时观察仍在运行，不能据此宣称两小时观察已完成。
- 发布与验证证据：`/var/tmp/opencoder-release-20260908-ui/`；生产观察由 `opencoder-release-observation-20260908.service` 执行。未接入运行的依赖分析草稿保留在工作区。

## 相关文档

- [Web 模块](../../../agents/web/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
