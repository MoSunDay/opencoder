Commit: (working-tree, 基于 b465f440)

# fleet console 手机视口横向溢出门禁：390px 全页走查 + 两处溢出修复

补上本轮 review 里唯一从未执行过的验收项。新增自包含门禁脚本，用 fixture API 托管**已提交的** `crates/web/spa/dist`，在 390x844 下驱动移动导航的每一个页面并测量横向溢出：

- `scripts/acceptance/spa_responsive.js`（399 行，新增）— HTTP fixture 服务 + Chromium 驱动 + 页内测量探针。走查 12 个页面（项目/{项目,进展,Owner 视角}、Agent/{大脑调度,全部执行,DAG 工作流,TODO 管理,团队组队,会话交互,Agent 配置}、节点/{节点列表,Env 管理}），并覆盖每个非激活 tab、每个"新建/创建"浮层与内联表单展开后的状态，共 27 次测量。退出码 0=全部贴合、1=溢出或页面不可达、2=harness 错误；截图落 `--shots`（默认 `/tmp/uitest/responsive`，按序编号前缀，因为中文页名归一化后会互相覆盖）。
- `scripts/acceptance/spa_responsive_fixtures.js`（125 行，新增）— 只读 GET fixture，形状逐一对齐各面板读取端（`/api/dag/*` 是裸数组、`/api/brain/capabilities` 是 `{capability,eng_inputs}` 行、`/api/project/overview` 是 goals→milestones→todos 三层等）；`ABSENT` 故意 404 若干端点以覆盖空态。

## 两条不显然的门禁语义（否则会假绿）

- **Chromium 移动模拟会在溢出时抬高 `window.innerWidth`**（390 → 413），`scrollWidth <= innerWidth` 因此恒真。判据取 `min(innerWidth, documentElement.clientWidth, visualViewport.width)` 作为视口边线，并上报越线元素。
- **`.fleet-content` 自身横向可滚不算合格**：移动导航就在该 pane 内，表格把 pane 撑宽时会把导航一起拖走。故除"祖先里有真正的横向滚动容器"这一豁免外，额外要求 `paneScroll <= paneClient + 1`。浮层测量前必须等 `[class*="-motion-"]` 消失，否则抽屉停在 `left=390` 会伪造一次溢出；每页 `pageerror` 直接判失败，因为崩掉的面板什么都不渲染、会空洞地通过。

## 找到并修复的两处溢出（均在 `src/app.css` 的 `@media (max-width: 767px)` 块）

1. **页头在每一页都溢出**：品牌区（163px nowrap）+ 5 个状态/身份/操作 chip 的 min-content ≈ 425px > 390px，单行必然撑破并触发上面的 innerWidth 抬高。改为 `height:auto !important; min-height:56px; line-height:1.5 !important; flex-wrap:wrap`（antd 的 cssinjs 内联了固定高度与 56px 行高，故需 `!important`），并让 `.fleet-header-side` 换行、其子项 `flex:0 0 auto` 停止把文字 chip 压成 12px 薄片。
2. **DAG 运行表撑宽 pane**：`src/dag/runsTable.jsx`（本轮禁改）未声明 `scroll`，列宽合计 ≈810px 且单元格 min-content 无法收缩（其余表格靠 `table-layout:auto` 收缩，故只有它越线）。门禁内唯一杠杆是 CSS：`.ant-table-content, .ant-table-body { overflow-x: auto }`，让表格自带横向滚动，与项目既有 `scroll={{ x: 'max-content' }}` 写法行为一致。**后续应交还** `src/dag/runsTable.jsx`：显式声明 `scroll={{ x: 'max-content' }}`，届时可撤掉这条兜底规则。

## 验证

- `node scripts/acceptance/spa_responsive.js` → exit 0，`visited 12 pages`、`SUMMARY measurements=27 overflowing=0`（修复前同一门禁 exit 1：先是全部 12 页 `doc=413/390`，再是 `Agent-DAG 工作流 tab:运行 pane=453/390 offenders=31`，两处修复分别让其转绿 —— 即门禁非空洞）。
- `cd crates/web/spa && npx vitest run` → 69 files / 556 tests passed。
- `bash scripts/check-spa-drift.sh` → `spa dist: no drift`（dist 已按 `npm run build` 重建，与当前工作树 src 一致；未手改 dist）。
- 未触碰禁改清单内任何文件；本轮自身改动仅 `src/app.css` + 重建的 `dist/` + 两个新脚本，均 ≤400 行、无 class、无凭据。
