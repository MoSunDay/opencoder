Commit: e3ca4fab32e4c08d0ac789dcc61b4a2d98c076fd

# 项目菜单与全局 Tag

用户要求项目分类下使用 Overview 菜单、Tag 独立放到菜单中，并增加所有项目、专项可共用的全局类型。保留当前工作区已有的 Overview / Tag 导航实现，补齐全局标签的界面、API、存储规则与验收。

- Tag 支持全局、项目、专项三种归属；全局无需选择项目或专项，未归属 TODO 也能选择全局标签。
- 同名时按专项、项目、全局的顺序使用。移动、删除或解除归属时按名称匹配剩余可用标签，不给原本未选标签的 TODO 自动添加标签。
- 沿用现有表和接口；`scope_type=global` 对应空字符串 `scope_id`，API 可省略该字段。全局名称唯一，非空归属和重复名称写入失败，不留下部分修改。
- 用户本次明确要求扩展原有范围限制，已同步[项目约定](../../../rules/07-project-module-contract.md)和[功能说明](../../project/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| Overview / Tag 导航与移动端选择 | `scopes the project category to Overview and the standalone Tag page` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 全局创建和切换类型 | `creates a global tag without projects or initiatives and omits the owner picker`、`converts project tags to global and requires a fresh owner when changing back` | `crates/web/spa/src/project/tags/tab.dom.test.jsx` |
| 未归属 TODO 创建与编辑 | `creates and edits unassigned TODOs with global tags` | `crates/web/spa/src/project/views/editing.dom.test.jsx` |
| 三层同名优先级和移除归属 | `shares global tags with every scope and applies initiative then project overrides` | `crates/web/spa/src/project/model/catalog.test.js` |
| 跨项目、局部覆盖、移动与事务回滚 | `global::*` | `crates/store/tests/project_tags/global.rs` |
| 非法选择校验 | `unassigned_todos_can_only_select_well_formed_global_tags` | `crates/store/src/project/tags.rs` |
| Web 的全局标签 API、冲突和删除 | `global_tags_allow_no_owner_and_validate_updates_and_duplicates` | `crates/web/tests/web_project_tags.rs` |
| Server 的全局范围切换和同名回退 | `global_tag_scope_moves_and_fallback_work_on_server` | `crates/control/tests/e2e/project_tags.rs` |
| 菜单、Tag 表格、类型切换与四种屏宽 | `tag_global_scope` 浏览器回执 | `scripts/acceptance/project_workbench_ui.js` |

## 实际验证

- `cargo test -p opencoder-store --test project_tags`：14 passed。
- `cargo test -p opencoder-web --test web_project_tags --test web_project_tag_moves`：6 passed。
- `cargo test -p opencoder-control --test e2e project_tags::`：2 passed，其余 239 个用例未运行。
- `cargo test -p opencoder-store --lib project::tags::tests`：2 passed，其余 64 个用例未运行。
- SPA 定向检查：8 个文件共 59 个用例通过。首次运行两个新增用例未等待表单更新，补齐等待后只重跑 `src/project/tags/tab.dom.test.jsx`，7 passed；其余 7 个文件的 52 个用例保持通过。
- SPA 命令：`npm test -- src/nav.test.js src/appNavigation.dom.test.jsx src/project/tags/model.test.js src/project/tags/tab.dom.test.jsx src/project/tags/panel.dom.test.jsx src/project/model/catalog.test.js src/project/views/editing.dom.test.jsx src/project/tests/todoDrawer.dom.test.jsx`。
- `npm run build`：通过，保留现有大包体积提示。
- `PROJECT_UI_ARTIFACTS=/tmp/opencoder-global-tags-ui-20261010 node scripts/acceptance/project_workbench_ui.js`：通过；1920、1280、768、390 屏宽，无浏览器异常。截图与回执位于指定临时目录。
- `cargo clippy -p opencoder-store -p opencoder-web -p opencoder-control --lib -- -D warnings`：通过，零警告。首次检查遇到工作区其他改动写入期间的编译版本不一致，确认源码完整后重跑通过。
- Rust 格式、改动文件行数、文档链接和差异空白检查通过。
- 只测试本次修改及其直接影响链路，未运行全量测试。以上为部署前验证，线上结果见下文。

## Web 发布结果

2026-10-10 已在公共 Web 生效。本次独立发布提交为 `e3ca4fab32e4c08d0ac789dcc61b4a2d98c076fd`；最终验收版本为接续的 `b490f40bc194ee00196baaaef1362a5202be8a8a`。接续提交只修改会话相关逻辑，已核验它包含本次提交，项目、标签实现和 SPA 摘要均保持一致。

- 隔离发布源码的定向检查：Store 14、Web 6、Control 2、SPA 8 个文件共 59 个用例通过；Clippy 零警告，SPA 构建和产物一致性检查通过。官方脚本构建的 7 个程序版本与校验和一致。
- 公共入口实测 Overview / Tag 菜单、全局类型无归属选择、真实浏览器保存、跨两个项目及未归属 TODO 复用全局标签。1920、1280、768、390 屏宽通过，无浏览器异常，临时项目、专项、TODO 和 Tag 已清理。
- 首次切换期间 34 次受理请求、246 次健康探测无失败；最大受理延迟 4.381 秒、最大调度间隔 10.803 秒，均满足 30 秒上限。
- 最终版本连续观察 902.68 秒：170 次 native 任务全部完成、877 次健康探测无失败；最大受理延迟 4.526 秒、最大调度间隔 11.204 秒、最大健康探测成功间隔 3.315 秒。
- 原完整发布演练在模型收尾等待 300 秒时超时；同一依赖任务随后自然完成，两个步骤均为 passed，长时间运行的 native 任务也正常结束。后补事件回放与持久化事件一致。原演练未执行完跨切换 SSE 重连断言，因此不将整套演练计作通过；最终回执单独列明功能、切换流量、原任务终态和连续观察结果。

证据位于 `/var/tmp/opencoder-project-global-tags/`：`final-result.json`、`final-observation.json`、`transition-verification.json`、`followup-source-verification.json`、`public-browser-followup/receipt.json`。原演练失败及后续恢复验证记录均保留。
