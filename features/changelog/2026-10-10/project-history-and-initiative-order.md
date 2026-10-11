Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# 概述历史入口与项目内专项排序

项目表、专项表及项目详情的专项表新增“概述历史”按钮，查看时不会同时打开行详情；沿用最近 10 个版本和回滚权限。专项名称列与所属项目列均为 300 像素，名称换行展示。

项目详情的专项支持手柄拖拽和键盘排序。前端保留筛选隐藏项，提交中禁止重复拖动，失败恢复原顺序。共享 `PUT /api/project/goals/:id/initiatives/order` 接收 `expected_ids` 和 `ids`，在事务中核对完整原顺序、成员集合及项目归属；并发变化返回 409。排序复用现有字段，不改变概述时间或版本。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 持久化、成员范围、并发顺序及概述保持 | `initiative_order_persists_and_rejects_stale_incomplete_or_foreign_lists` | [存储测试](../../../crates/store/tests/project_initiative_order.rs) |
| 中途写入失败整组恢复 | `initiative_order_write_failure_rolls_back_every_row` | [存储测试](../../../crates/store/tests/project_initiative_order.rs) |
| HTTP 保存、错误状态与权限 | `initiative_order_http_persists_checks_conflicts_and_requires_write_permission` | [接口测试](../../../crates/control/tests/e2e/project_api/order.rs) |
| 隐藏项、双向拖动与无效目标 | `moves in either direction and preserves hidden initiative order without mutating input` | [纯函数测试](../../../crates/web/spa/src/project/sorting/model.test.js) |
| 筛选提交、重复提交、失败恢复、只读与三个历史入口 | `saves a filtered drag...`、`restores the original order...`、`opens history from %s...` | [交互测试](../../../crates/web/spa/src/project/sorting/initiatives.dom.test.jsx) |
| 四种屏宽、真实保存、刷新保留、鼠标及键盘拖动 | `project_order_ui.js` | [独立 Server 浏览器验收](../../../scripts/acceptance/project_order_ui.js) |

范围与操作入口见 [项目工作台](../../project/index.md)、[验收说明](../../../scripts/acceptance/project/README.md)。

## 定向验证

- SPA：显式运行排序纯函数与交互、概述历史、表格筛选、项目详情及归属编辑 6 个相关文件，共 19 个用例最终通过。
- `cargo test -p opencoder-store --test project_initiative_order`：2 passed。
- `cargo test -p opencoder-control --test e2e project_api::order`：1 passed。
- `cargo clippy -p opencoder-store -p opencoder-web -p opencoder-control --all-targets -- -D warnings`：零警告。
- Rust 格式及 `git diff --check` 通过；未运行全量测试。
- `npm run build` 与 `cargo build -p opencoder-server -p opencoder-cli` 通过；Server 和 ctl 的 SPA 摘要与最终页面产物一致。
- 独立真实 Server 的 `project_order_ui.js` 验收通过：1920、1280、768、390 像素列表、项目详情和历史抽屉；鼠标和键盘排序、隐藏项保留、失败恢复、刷新持久化均通过，浏览器异常为 0。未部署。
