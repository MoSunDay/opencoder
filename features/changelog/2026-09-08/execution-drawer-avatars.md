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

- SPA：`npm test` → 53 个文件、467 个测试通过；`npm run build` 通过（保留已有单包体积提示）。
- 浏览器：1600 / 1024 / 390px 视口中，抽屉实测宽度分别为 1200 / 768 / 292.5px；角色头像和展开/关闭操作通过，无页面运行错误。
- `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo build --workspace` 通过。
- `cargo test --workspace` 首次在 `nodes_smoke_proc` 因节点容量门槛失败；按既有验收环境使用 `/var/tmp` 并排除回环代理后，`cargo test -p opencoder --test nodes_smoke_proc` 复测 1 passed。未取得本轮 Rust 全量通过结果。

## 相关文档

- [Web 模块](../../../agents/web/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
