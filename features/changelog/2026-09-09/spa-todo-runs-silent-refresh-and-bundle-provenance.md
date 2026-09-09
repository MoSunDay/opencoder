Commit: (working-tree, 基于 7d873ea6)

# fleet console TODO 运行视图接入 loading 约定 + 门禁产物溯源 + 漂移检查去假阳性

收 7d873ea6 评审的 P1-1 / P1-2 / P2-1 / P2-3，外加验证过程中实测出来的一条新缺陷
（`scripts/check-spa-drift.sh` 会对忠实的 `dist/` 报假 DRIFT）。全部改动只碰 SPA 源码、
node 门禁脚本与文档，零 Rust 变更。

## 1. `todoRunsPanel.jsx` 两张表接入 `ui/tableLoading.js`（P1-1 + P2-3）

评审认定这是上一轮唯一漏接约定的表，且后果比 `fleet/executions.jsx` 更重：`onRow` 的行点击
（选中工作流 → 右侧渲染 `WorkflowDetail`）是该面板**唯一的主交互**，而遮罩会给
`.ant-spin-container` 上 `opacity .5 + pointer-events: none`。

- 外层「工作流」表：`loading={tableLoading(loading)}` + `dataSource={tableRows(loading, rows)}`
  + `scroll={{ x: 'max-content' }}`（P2-3：此前它是唯一不声明 `scroll` 的列表表，桌面宽度上
  没有任何门禁会抓它，只有 390 这一档）。首屏在途时不再渲染空态占位 —— antd 只在
  `dataSource` 与其内部 `EMPTY_LIST` 同引用且 `spinning` 时抑制占位，`rows=[]` 会一边拉取
  一边断言「暂无数据」。
- 内层「TODO 项」表（`WorkflowDetail`）：补 silent 通道 —— `load(silent)`，挂载 `load(false)`，
  `interrupt`/`resume`/`cancel` 与 SSE 终帧刷新走 `load(true)`；同样接入两个函数，并加
  `className="oc-todo-items"` + `scroll`（className 是为了 DOM 测试能把两张表的遮罩分开断言）。
  上一轮把这张表记成「需要先理清 wf-A/wf-B 双工作流状态才能迁」，实测不需要：外层早有
  `load(silent)`（3s 轮询就是 `load(true)`），内层只是同一形状的几行改动，双工作流状态与
  loading 语义无关。
- `onTerminal` 用 `useCallback(() => load(true), [load])` 而非内联箭头：`EventsFeed` 的 effect
  依赖 `[workflowId, onNotice, onTerminal]`，内联箭头每次渲染换身份，而 3s 轮询每轮都会以新的
  `summary` 重渲染 `WorkflowDetail` → 每轮 abort 重订阅 SSE 并清空事件列表。
- `todoPanel.dom.test.jsx` 的行选择器 `tbody tr` → `tbody tr.ant-table-row`：`scroll.x` 打开后
  rc-table 会在 tbody 首行插入 `aria-hidden` 的 `ant-table-measure-row`，旧选择器点到的是它。

## 2. 悬空变量扫描自证（P1-2）

`theme.test.js` 的 `srcFiles` 走 `readdirSync(..., { recursive: true })` + 三重过滤；任一环在
Node 行为变化下失效（`recursive` 回 Dirent 而非 string、路径分隔符变化），`srcFiles` 就变成
`[]`，扫描退回「只扫两张样式表」而测试**照过** —— 与本轮主题（守卫不得空洞）同一类漏洞。
补三条自证断言：`srcFiles.length > 0`、至少覆盖一个 `.jsx`、`--oc-mono` 必须被归因到
`ui/mono.js`（纯 JS 模块里的 `MONO_VAR`，样式表无法替代它，故能证明递归确实读到了 JS 源）。
同时把分隔符归一化（`split(/[\\/]/).join('/')`）提到过滤之前。变异验证：把走查降级为 Dirent
形状 → 三条断言转红；健康走查覆盖 174 个 `.js/.jsx`。

## 3. 门禁产物溯源（P2-1）

`scripts/acceptance/spa_responsive.js` 的头注原写「Serves the *committed* SPA bundle」，实际
`SPA` 指向**工作树** `crates/web/spa/dist`。当前工作树 dist 已重建未提交，于是新克隆在 HEAD
上跑这个门禁会 exit 1，报的正是上一轮声称已修的溢出 —— 失败是响的，但归因误导。

- 头注订正 + 新增 `Provenance:` 段：明确它测的是工作树，不是 HEAD。
- 启动即打印一行溯源：`bundle: <abs dist> (working tree; dist differs from HEAD: N path(s), src differs from HEAD: M path(s))`
  / `(clean: dist == HEAD, no uncommitted src)` / `(git unavailable: provenance unknown)`；
  `src` 脏而 `dist` 干净时额外警告「被测产物不含这些源码改动」。git 缺失/非仓库降级为提示，
  不阻断（门禁要能在 tarball 里跑）。
- `--require-committed`：产物不等于 HEAD 就 exit 2 拒测；`--drift`：先跑
  `scripts/check-spa-drift.sh`（`spawnSync` + `stdio: inherit`，失败 exit 2，脚本缺失/不可执行
  则打印 skip 后继续）。测量逻辑、fixture、390×844 视口未动，退出码语义不变（2 = harness 错误）。

## 4. `check-spa-drift.sh` 对忠实产物报假 DRIFT（本轮实测新发现）

验证 `--drift` 时撞上：`dist/` 明明是刚 `npm run build` 出来的，漂移检查仍报 DRIFT。实测
同一 `src/` 连续 4 次 `npm run build`：3 次 `app.js` md5 相同、1 次不同；首个差异字节在
**偏移 4**（第一个被压缩器重命名的标识符），尺寸差 44 字节，是一次全局命名级联；
`app.css`/`index.html` 每次都字节一致。即 vite/esbuild 的压缩器命名在本仓库并非位稳定
（`css` 稳定、`js` 偶发变体），单次 byte-diff 因此约 1/4 概率冤枉一个忠实的 `dist/`。

修法：只在差异**局限于** `static/app.js` 时重建重试（最多 3 次构建），其余情况（`app.css`、
`index.html`、`Only in` 多/少文件）立即判 DRIFT。真实的 `src` 改动会改变语义、每次构建都差异，
所以重试只可能去掉假阳性，不可能掩盖漂移；失败时的 diff 输出限 40 行、每行限 200 字符
（压缩产物 diff 是 MB 级的单行，只限行数仍会倾倒上百 MB）。

## 5. 上一节自己踩的坑：`head` 截断把判决吞掉（提交后验证时发现）

`bc82d428` 里那段截断写作 `printf '%s\n' "$out" | head -40`。`head` 读满 40 行即退出，`printf`
随即收到 SIGPIPE，`set -o pipefail` 把管道判为失败，`set -e` 于是在打印判决**之前**中止脚本：
实测 `bc82d428` 干净 worktree 上（HEAD 的 `dist/` 相对 HEAD 的 `src/` 是陈旧产物，`app.css` 与
`app.js` 同时差异，即评审说的"新克隆上误报 overflow"场景）退出码是 **141** 而不是 1，
`spa dist: DRIFT detected — run scripts/build-spa.sh` 一行也没打出来。

第 4 节的自测之所以没抓到：当时是往 `dist/static/app.css` **手工注入一行**，diff 只有几十行、
塞得进管道缓冲区，`printf` 写完就退出，构不成 SIGPIPE。只有 MB 级的真实陈旧产物才会触发。
改成 `cap_diff()`：`awk` 读完整条 stdin（不留人握着断管），在 awk 内部同时做行数与行宽截断。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 首屏在途不撒谎「暂无数据」 | `首屏在途时不宣称暂无数据，兑现后行照常渲染` | `crates/web/spa/src/todoRunsPanel.dom.test.jsx`（新增） |
| 遮罩断言非空洞（选择器活着） | `首屏拉取越过 SPIN_DELAY_MS 才亮遮罩，兑现后遮罩消失（证明选择器是活的）` | 同上 |
| 行点击选中 + 详情首拉不撒谎 | `点击行选中工作流；详情首拉在途时 TODO 项表同样不撒谎` | 同上 |
| 变更后双表静默刷新（P1-1 核心） | `中断后的双表刷新静默：越过 SPIN_DELAY_MS 也不遮罩，行与按钮都还在` | 同上 |
| 3s 轮询静默 | `3s 轮询静默：挂住的 poll 不点着任何一张表的遮罩` | 同上 |
| 悬空变量扫描覆盖到 JS 源（P1-2） | `leaves no var(--oc-*) reference dangling across app.css, project.css and every src file`（新增 3 条自证断言） | `crates/web/spa/src/theme.test.js` |
| 390×844 溢出门禁 + 产物溯源 | `node scripts/acceptance/spa_responsive.js`（12 页 / 27 测量 / 0 溢出）；`--require-committed` exit 2；`--drift` 后 PASS | `scripts/acceptance/spa_responsive.js` |
| 漂移检查去假阳性 | 三条路径实跑（见验证） | `scripts/check-spa-drift.sh` |

变异验证（逐条改回原状后确认转红，再复原）：`interrupt` 的 `load(true)` → `load(false)` 只有第 4 条转红；
外层 `dataSource={tableRows(...)}` → `{rows}` 只有第 1 条转红；轮询 `load(true)` → `load(false)` 只有第 5 条转红；
内层 `dataSource` 同理只有第 3 条转红。

## 验证

- `cd crates/web/spa && npx vitest run` → **70 files / 561 tests passed**（39.6s）。
  本轮增量 = **+1 文件 / +5 tests**（`todoRunsPanel.dom.test.jsx`），另在 `theme.test.js` 既有用例内
  加 3 条断言（`it` 数不变）。起始工作树基线 69 files / 556 tests —— 该基线本身含并行会话在途
  未提交的测试文件，故 556→561 的差额可全部归因本轮，而 541→556 不能（见「订正」）。
- 定向：`npx vitest run src/todoRunsPanel.dom.test.jsx src/todoPanel.dom.test.jsx src/ui/tableLoading.dom.test.jsx src/envsPanel.dom.test.jsx` → 4 files / 22 tests passed；
  `npx vitest run src/theme.test.js src/ui/mono.test.js` → 2 files / 28 tests passed。
- `node scripts/acceptance/spa_responsive.js --shots /tmp/uitest/responsive-r2` → exit 0，
  `visited 12 pages`、`SUMMARY measurements=27 overflowing=0`（含 `Agent/TODO 管理 tab:运行`，
  即本轮改动的面板）。
- `node scripts/acceptance/spa_responsive.js --drift ...` → `spa dist: no drift (build 1/3)` →
  `drift: OK` → exit 0；`--require-committed` → 打印溯源行后 exit 2
  （当前工作树：`dist differs from HEAD: 2 path(s), src differs from HEAD: 16 path(s)`）。
- `bash scripts/check-spa-drift.sh` 三条路径，均在 `bc82d428` 的独立 worktree 里跑（不碰并行会话
  的在途源码）：
  1. HEAD 陈旧 dist（`app.css`+`app.js` 同时差异，MB 级）→ 首次构建即 exit **1**，输出 4553 字节
     / 47 行，末尾 `... diff truncated to 40 lines` 与 `DRIFT detected — run scripts/build-spa.sh`
     判决齐全（`bc82d428` 的 `head` 版在此处是 exit 141 且无判决）；
  2. worktree 内 `npm run build` 出忠实 dist → `no drift (build 1/3)` exit **0**；
  3. 往 `src/main.jsx` 追加 `window.__ocDriftProbe = 1;`（真实语义改动、只动 `app.js`）→
     `only static/app.js differs — rebuilding (1/3)`、`(2/3)` → `DRIFT detected — static/app.js
     differs on all 3 builds` exit **1**，输出 1794 字节。重试路径确实拦不住真漂移。
  worktree 已 `git worktree remove`，主树 dist/ 未被这些实验触碰。
- `cargo build -p opencoder-web` → Finished（`dist/` 走 `include_bytes!` 内嵌，产物变化必须过构建）。
- `cargo test -p opencoder-web` → **291 passed / 0 failed**（59 个测试二进制，EXIT=0）。该 crate 的
  `src/auth_mw.rs`、`src/lib.rs` 属并行会话在途改动，故数字是混合溯源；本轮零 Rust 变更，跑它只为
  确认重建后的 `dist/` 经 `include_bytes!` 内嵌后 HTTP/SSE 契约测试仍全绿。
- 行数 gate：新增 `todoRunsPanel.dom.test.jsx` 247 行（≤400）；迭代中文件最大
  `spa_responsive.js` 510 行、`todoRunsPanel.jsx` 326、`theme.test.js` 231、`check-spa-drift.sh` 81（均 ≤800）。
  无 class、无新增依赖、无硬编码凭据。
- 订正（评审 P2-2，rules/02 证据纪律）：7d873ea6 的 changelog 写「整改前基线 541 → 556」，
  暗示 +15 全属该轮；逐文件比对 `git show HEAD^/HEAD` 后该轮自身 `it(` 增量为 **+14**
  （theme 2、mono 1、envsPanel.dom 1、tableLoading 5、tableLoading.dom 5），556 是**工作树**数字，
  含并行会话在途未提交的测试文件。已在原 changelog 就地标注。

## 交接 / 未完成

- **`dist/` 与 `<App component={false}>` 仍未入库（评审必须项 ②）**：并行会话的 SPA 源码
  （`main.jsx` 权限化 IA、`admin/`、`operators/`、`nav.js`、`store.js`、`login.jsx`、
  `fleet/detail.jsx`）本轮期间仍未提交（`src differs from HEAD: 16 path(s)` 中 12 条属它），
  提交重建产物 = 把别人未提交的源码烘进已提交 artifact。`<App component={false}>` 已存在于
  工作树 `main.jsx:173` 但与该会话的改动无法干净拆分。落地后：`scripts/build-spa.sh` →
  提交 `dist/` → `node scripts/acceptance/spa_responsive.js --require-committed` 即可自证测的是 HEAD。
- **workspace 回归 gate 本轮仍无绿色记录（rules/02）**：`git status` 有 50 个 `.rs` 属并行会话
  在途改动（control/store/worker/core），`cargo test --workspace` + clippy 跑了也是在测它们的代码，
  结果无法归因本轮（本轮零 Rust 变更）。与上一轮同一卡点，需在并行会话落地后补跑。
- `api.js` 无 timeout / 不传 `AbortSignal`：一次永不 settle 的**非静默**请求仍会让 loading 永久为真
  → 永久遮罩。`tableLoading` 只是让遮罩诚实，没有给它上限；按计划留作独立一轮。
- 门禁档位仍只有 390×844：`.ant-table-content { overflow-x: auto }` 是 767px 以下的安全网，
  桌面宽度上漏写 `scroll={{x}}` 的表只有靠本轮补齐的声明来约束。若要加档，768px（媒体查询
  边界第一档）需要给门禁补桌面导航驱动路径（当前只会走 `.fleet-mobile-nav`）。
