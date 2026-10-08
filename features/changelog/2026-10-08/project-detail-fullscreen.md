# 专项与 TODO 详情全屏展示

专项详情去掉 1200px 宽度限制，TODO 详情去掉 1000px 外层宽度限制。两类详情沿用右侧抽屉，在各屏宽下覆盖整个视口；关闭、看板操作和执行入口沿用现有行为。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 项目进入专项，再进入 TODO，两层抽屉均全宽 | `opens project progress, then the initiative board and TODO in full-width right-side drawers` | `crates/web/spa/src/project/project.dom.test.jsx` |
| 四种屏宽下两类抽屉的实际视口边界、关闭和返回 | `PASS fullscreen initiative and TODO drawers` | `scripts/acceptance/project_workbench_ui.js` |

- SPA 全量：134 个测试文件，980 个测试通过。
- Chromium：1920、1280、768、390 四种宽度，共 8 组抽屉宽高测量通过；看板拖动、分组同步、失败回退及项目、Tag 增删改通过，页面异常为 0。
- Rust 全量：`cargo test --workspace` → 5686 passed / 0 failed；既有 ignored 7。
- `cargo build --workspace --bins --examples`、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 通过。
- 发布脚本回归：rolling 66 项、signal 12 项通过；SPA 产物漂移检查通过。
