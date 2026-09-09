Commit: (working-tree, 基于 64f7000f)

# fleet console 浅色主题精修 · 评审整改：表格 loading 诚实化 + 守卫加固 + 等宽收尾

针对 e488f767 / 64f7000f 的评审报告（净完成度约 75%）执行整改。三笔债务分别处理：
①「loading 把纯视觉需求做成了带 `pointer-events` 副作用的交互变更」；②「新增守卫没跟上新增变量」；
③「阶段 ⑤ 的响应式验收从未执行」（见同目录 `spa-phone-viewport-overflow-gate.md`）。

## 1. 表格 loading 语义收敛为单一约定

新增 `src/ui/tableLoading.js`（纯数据/纯函数，不引 React）：`SPIN_DELAY_MS = 200`、
`tableLoading(spinning) -> { spinning, delay }`、`tableRows(spinning, rows) -> undefined | 同一数组引用`。
两条 antd 6 实现事实决定了它存在：

- `es/table/hooks/useSpinProps.js`：裸 boolean 会被当成 `delay: 0`，于是一次 <100ms 的同机刷新也会闪一下遮罩；
- `es/table/InternalTable.js`：空态占位只有在 `spinProps.spinning` 为真且 `dataSource` 与内部 `EMPTY_LIST`
  同引用（`undefined`/`null`）时才被抑制。`dataSource={[]}` 会一边拉取一边断言「暂无 …」——首屏对用户撒谎。

接线到本轮新增 loading 的五张表（`fleet/executions.jsx`、`fleet/nodes.jsx`、`fleet/teams.jsx`、
`envsPanel.jsx`、`dag/runsTable.jsx`），并修掉评审列为「本提交亲手引入的交互回归」的一处：

- `fleet/executions.jsx`：表格由 `loading={loading || loadingMore}` 改回只看 `loading`。append（翻页）
  期间遮罩会给 `.ant-spin-container` 上 `opacity:.5 + pointer-events:none`，ID 链接当场点不动，
  还与「加载更早的执行」按钮自带的 spinner 撞成两个；`loadingMore` 归还给该按钮。
- `envsPanel.jsx`：`loadEnvs(opts)` 增加 `silent`（仿 `nodes.jsx`）。新建/保存/删除后的刷新走 silent，
  否则每次增删改都会把整表连同各行「编辑/删除」一起变暗且不可点（迁移前这些刷新是无感的）。
- `fleet/nodes.jsx` `onSaved={() => load(false)}`、`fleet/teams.jsx` `onClick={() => load()}`：
  位置布尔/事件对象再也不会被误读成 `silent`。
- `dag/runsTable.jsx` 补 `scroll={{ x: 'max-content' }}`，与其余列表表一致（响应式门禁暴露的越线项）。

## 2. 守卫加固：让新增变量不再「改了就静默漂移、构建仍全绿」

- `theme.test.js` 的 `-rgb` 推导守卫由硬编码 `--oc-primary` 改为**遍历全部 `--oc-*-rgb`**，逐个要求其
  `#rrggbb` 孪生存在且三元组由孪生推导。此前把 `--oc-accent-user` 两侧同步改掉而漏改 `-rgb`，
  26 项测试全绿、RoleAvatar 会在青色淡底上画蓝绿字形。
- 悬空变量扫描由「只读 app.css」扩展到 `app.css` + `project/project.css` + `src/` 下**全部 `.js`/`.jsx`**，
  且要求引用同时在 `:root` 与 `theme.js cssVars` 中存在。`transcript.jsx` 的 4 处 inline 引用不带 fallback，
  删掉变量会让 `rgba(var(--oc-accent-user-rgb), .1)` 计算值非法（淡底静默消失）而构建全绿。
- 新增断言：`theme.token.fontFamilyCode === MONO === cssVars['--oc-mono']`。
- `ui/mono.test.js` 的首条测试原是恒真断言（`theme.js` 里就是 `'--oc-mono': MONO`，等于 `MONO === MONO`），
  改为 `readFileSync` 直读 `app.css` 的 `:root` 声明后比较，并补 antd code token 一侧。
  三处变异验证均转红后复原：app.css 单侧改 `--oc-accent-user-rgb`、jsx 里引用 `var(--oc-nope)`、
  从 `--oc-mono` 删掉 `'Liberation Mono'`。

## 3. 等宽与调色板收尾（评审「应当/可以」项）

- `theme.js`：`accentUser`/`accentAi`/`accentWasm` 收进 `palette`（此前每个 hex 在文件内写了两遍），
  `cssVars` 走单源；新增 `--oc-accent-wasm`（`app.css` `:root` 同步声明，`.dag-edit-node--wasm` 改用变量，
  此前该规则紧邻的上一行已经用 `var(--oc-primary)`）；`token.fontFamilyCode: MONO` —— 否则 antd 默认代码栈
  （含 `Courier`、缺 `ui-monospace`）统治 `<Text code>`（`dag/runsTable.jsx`、`agentsConfig.jsx`、
  `admin/usersDrawer.jsx`）。
- `project/project.css`（上一轮整文件未触碰）：`.md-body` 文字色 → `var(--oc-text)`、`.md-body a` →
  `var(--oc-primary)`、`.md-body code` 的手写窄栈 → `var(--oc-mono)`。最后一条是用户可见缺陷：
  同一条消息在流式结束前后会换字体（Linux 上窄栈缺 `Liberation Mono`，落到 DejaVu）。
- `agentNfsCard.jsx` 裸 `<code>` 补 `MONO_VAR`（此前无任何字体声明命中，走 UA 默认）。
- `app.css` `.oc-row-selected`：加作用域（`.oc-todo-runs`，`todoRunsPanel.jsx` 补同名 `className`）
  并补 `:hover` 孪生。`!important` 本身必要（antd 行背景/hover 规则压过裸类选择器，而点击选中的表格
  指针必然停在选中行上），但它同时压掉了选中行自己的 hover 反馈；选中淡底提到 0.10、hover 0.16，
  与 `rowHoverBg: #f5f9ff` 拉开可辨距离。
- `transcript.jsx` `EmptyHint`：去掉外层 48px padding（antd `Empty` 自带 `marginBlock:32`，双重留白约 230px），
  改为 `marginBlock: 24` 的单一间距决策。
- 删除死代码 `src/ui/monoText.jsx`：全树零 importer、自身无测试；实际使用的原语是 `ui/mono.js` 的 `MONO_VAR`
  （约 32 处调用点）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 约定本体（delay/形状/引用同一性） | `spin delay is 200ms` 等 5 项 | `crates/web/spa/src/ui/tableLoading.test.js` |
| append 不遮表且 ID 链接仍可点 | `keeps the execution index unmasked and clickable while 加载更早的执行 appends` | `crates/web/spa/src/ui/tableLoading.dom.test.jsx` |
| 3s 静默轮询永不遮罩 | `never masks the node table on the silent 3s poll, so row actions stay clickable` | 同上 |
| reject 后 loading 必清 | `clears the mask after a rejected fetch …` | 同上 |
| 首屏不撒谎 | `does not claim 暂无 Opencoder 节点 while the first fetch is still unknown` | 同上 |
| delay 200 消除快刷新闪屏 | `hides a refresh shorter than the spin delay and only masks a still-pending one` | 同上 |
| env 变更后静默刷新不遮表 | `keeps the env table unmasked while the post-delete silent refresh is in flight` | `crates/web/spa/src/envsPanel.dom.test.jsx` |
| 全部 `-rgb` 孪生推导 | `derives every --oc-*-rgb from its hex twin (rgba() literals stay honest)` | `crates/web/spa/src/theme.test.js` |
| 悬空变量扫描（css + 全部 js/jsx） | `leaves no var(--oc-*) reference dangling across app.css, project.css and every src file` | 同上 |
| antd 代码字体 = MONO = --oc-mono | `pins the antd code face to the same stack as --oc-mono` | `theme.test.js` / `ui/mono.test.js` |
| 等宽栈与 app.css 声明一致（非恒真） | `is the same stack the --oc-mono custom property declares in app.css` | `crates/web/spa/src/ui/mono.test.js` |
| 390×844 横向溢出门禁 | `node scripts/acceptance/spa_responsive.js`（12 页 / 27 测量 / 0 溢出） | `scripts/acceptance/spa_responsive.js` |

## 验证

- `cd crates/web/spa && npx vitest run` → **69 files / 556 tests passed**（整改前基线 541）。
- `node scripts/acceptance/spa_responsive.js` → exit 0，`visited 12 pages`、`SUMMARY measurements=27 overflowing=0`。
- `bash scripts/check-spa-drift.sh` → `spa dist: no drift`（工作树 `dist/` 已按 `npm run build` 重建）。
- `cargo build -p opencoder-web` → EXIT=0（`dist/` 由 `include_bytes!` 内嵌，产物变化需过构建）。
- 行数 gate：新增文件 ≤ 399 行（`spa_responsive.js` 399、`tableLoading.dom.test.jsx` 188、
  `spa_responsive_fixtures.js` 125、`tableLoading.test.js` 37、`tableLoading.js` 30）；
  迭代中文件最大 `app.css` 513 行（上限 800）。无 class、无新增依赖、无硬编码凭据。

## 交接 / 未完成（刻意为之）

- **`dist/` 未提交、`main.jsx` 的 `<App component={false}>` 仍未入库**：与上一轮同一原因——并行会话的
  SPA 在途改动（`main.jsx` 权限化 IA + `admin/usersDrawer.jsx`、`operators/`、`nav.js`、`login.jsx`、
  `store.js`、`fleet/detail.jsx`）尚未落地，提交重建后的 `dist/` 会把未提交源码烘进已提交产物，违反
  dist↔src 契约；`<App>` 与 `UsersDrawer` 在 JSX 上互相嵌套也无法干净拆分。后果已知且无回归：
  22 处 `useMessage()` 暂走静态回落分支，「toast 跟随主题与 zh-CN locale」的收益要到并行会话落地才生效。
  落地后执行 `scripts/build-spa.sh` 并提交 `dist/` 即可一并收敛。
- `api.js` 无 timeout、调用方不传 `signal`：一次永不 settle 的**非静默**请求会让 loading 永久为真，
  配合 `pointer-events:none` 使行内操作永久不可点（评审 4.8）。本轮未动 `api.js`（影响面覆盖全部面板），
  留作独立一轮：`apiGet` 加 `AbortSignal.timeout` 或给列表拉取传 `signal`。
- `todoRunsPanel.jsx` 的两张表仍用裸 `loading` 布尔，且其 wf-A/wf-B 双工作流状态存在「A 的 `finally`
  清掉 B 的 loading」的既有缺陷（评审 4.12）。迁到新约定需要先把双工作流状态理清，本轮只借它加了
  `className="oc-todo-runs"` 作用域。
- `chatSidebar.jsx` 的 264px 固定宽（评审明确推迟项）与 `chat.jsx:438` 裸 `rgba(0,0,0,.08)` 阴影未动。
