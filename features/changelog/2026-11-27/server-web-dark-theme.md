# Web SPA 深色主题：统一 `--oc-*` 色板、antd 主题配置与原始 CSS 面（纯视觉，无接口 / 状态 / 事件 / 流程变更）

一句话：服务端 Web SPA 从「浅色画布 + 白色面板 + 默认蓝」整体切换为深色参考色板——画布 `#0d1117`、
面板 `#161b22`、填充 `#21262d`、边框 `#30363d`、主色 `#58a6ff`、状态色 `#3fb950` / `#d29922` /
`#f85149`，基准字号 13px、头部 48px、内容间距 16px、1px 边框承担分层、面板不加柔和阴影、主圆角 6px。
`theme.js` 的 `palette` / `tints` 与 `styles/tokens.css` 的 `--oc-*` 一一对应并由 `theme.test.js` /
`mono.test.js` 锁步，组件局部样式里的浅色字面量全部改为 token 引用；`app.css` 为遵守行数限制按职责拆为
`styles/tokens.css` / `styles/shell.css` / `styles/canvas.css`。不改任何接口、状态、事件与流程，既有
响应式契约原样保留。

## 背景与决策

- 问题：SPA 的原始 CSS 面按浅色前提写着颜色字面量（画布 `#f4f6f8`、面板 `#fff`、主色 `#1677ff`、
  `rgba(0, 0, 0, …)` 灰阶），antd 组件另由 `theme.js` 配置；两处分别维护，改色需人工对齐，各组件目录下
  还散落各自的十六进制 / rgba 字面量与 `var(--oc-*, <浅色>)` 回退。
- 决策：只做视觉层替换。所有共享色集中在 `theme.js::palette`（antd 侧）与 `styles/tokens.css`
  （CSS 侧），二者由测试锁步；其余样式表只允许引用 token。

| 维度 | 取值 | 落点 |
|------|------|------|
| 画布（页面底） | `#0d1117` | `palette.bgLayout` / `--oc-bg-layout` |
| 面板（头部 / Sider / 卡片 / 浮层） | `#161b22` | `palette.panel` / `--oc-panel-bg` |
| 浅填充（表头 / 标签 / Tag） | `#21262d` | `palette.fillSubtle` / `--oc-fill-subtle` |
| 细边框 / 控件与节点边框 | `#30363d` / `#484f58` | `--oc-border` / `--oc-border-strong` |
| 主色 | `#58a6ff` | `palette.primary` / `--oc-primary`（含 `-rgb` 与 15% / 25% / 40% 色阶） |
| 状态色 | 成功 `#3fb950` / 警告 `#d29922` / 错误 `#f85149` | `--oc-success` / `--oc-warning` / `--oc-error`（各带 `-rgb` 与 15% 底色） |
| 文字 | 正文 `#e6edf3` / 次要 `#8b949e` / 三级 `#6e7681` | `--oc-text*`（三级保持不透明 hex，不用 antd 深色默认的 rgba） |
| 字号 | 基准 13px，SM 12px | `FONT_SIZE` / `--oc-font-size-base` |
| 字体 | 正文栈带 CJK 回退；等宽栈 Latin 等宽在前、CJK 回退在后、generic 兜底 | `SANS` / `MONO` ↔ `--oc-font-sans` / `--oc-mono` |
| 头部 | 48px 高、`0 16px` 内边距 | antd `Layout.headerHeight` / `headerPadding`；`shell.css` `.fleet-header` |
| 内容间距 | 16px | `.fleet-content` padding、`.fleet-header` gap、`.oc-page-header` 下边距 |
| 边框与阴影 | 1px 边框承担全部分层；面板阴影 `none`；只有浮层（弹窗 / 下拉 / 选择器 / popover）保留一层深色投影，抽屉靠细边框分隔 | `shadowPanel` / `shadowFloating` / `dropShadowPopover` ↔ `--oc-shadow-panel` |
| 圆角 | 主 6px；Tag 4px；浮层 LG 8px；Card 回到 6px | antd `borderRadius*` |
| 原生控件 | `color-scheme: dark` | `tokens.css` `:root` |

- 不改的：导航与页面注册、事件处理、store、请求与 SSE、路由、DAG / 计划的数据结构均未触及；响应式契约
  沿用原断点（<768 隐藏 Sider 改为分类 tabs + Select、sheet 底部停靠、移动端内容 padding 收缩），只替换
  颜色、字体、圆角与阴影。
- 行数约束：基础层 + 外壳 + 画布规则同放一个 `app.css` 会超过迭代文件行数上限，因此按职责拆成
  `styles/tokens.css`（token）、`styles/shell.css`（外壳）、`styles/canvas.css`（画布）三张表，`app.css`
  只保留基础层并在文件头 `@import`（CSS `@import` 必须置顶；Vite 构建仍内联为单个 `dist/static/app.css`，
  层叠顺序 token → shell → canvas → 基础层）。

## 行为与实现

- `crates/web/spa/src/theme.js`：`palette` 扩为全部共享色并新增 `tints`（`withAlpha` 由 hex 派生色阶，
  不再手写第二份）；`algorithm = [darkAlgorithm, pinReferenceHues]`（antd 深色算法之后把 info / link 钉回
  主色）；`SANS` 带 CJK 回退、`FONT_SIZE = 13`、`fontSizeSM = 12`、`fontFamilyCode = MONO`；
  `shadowPanel = 'none'`，`shadowFloating` 只进浮层 token；组件 token：Layout 48px 头部、Menu pill 选中、
  Table 无表头分割线、Card 16px 内边距、Tag SM 4 / 主 6 / 浮层 LG 8 的圆角阶梯。导出 `palette` / `tints` /
  `SANS` / `FONT_SIZE` / `shadowPanel` / `shadowFloating` / `dropShadowPopover` / `algorithm` 供测试断言。
- `crates/web/spa/src/ui/mono.js`：`MONO` 追加 CJK 回退（`'Noto Sans Mono CJK SC'`、`'PingFang SC'`、
  `'Microsoft YaHei'`），与 `--oc-mono` 同序；`MONO_VAR` 仍解析自该自定义属性、以裸 `monospace` 兜底。
- `crates/web/spa/src/styles/tokens.css`（新）：`:root` 内全部 `--oc-*` 声明（含 `-rgb` 孪生与状态底色）；
  `styles/shell.css`（新）：头部 / 品牌 / Sider / Menu / Content / 统一页头 / sheet / 移动端导航与外壳
  响应式规则，颜色只引用 token；`styles/canvas.css`（新）：DAG 运行节点 / 编辑器外壳 / React Flow
  （`--xy-edge-stroke*`）与 CodeMirror 外观，节点 / 连线 / 把手的几何像素值不变。
- `crates/web/spa/src/app.css`：617 → 216 行，三条 `@import` 置顶；保留滚动条、焦点环、内容面、抽屉、
  Markdown 覆盖与表格兜底；antd 默认无边框的浮层（`.ant-modal-container` / `.ant-dropdown-menu` /
  `.ant-select-dropdown` / `.ant-picker-panel-container`）统一补 1px 边框，并使用 antd 6 的面板类名。
- `crates/web/spa/src/brain/workbench/milestone/canvas.jsx`：去掉连线内联 `stroke`，颜色改由 canvas.css
  的 `--xy-edge-stroke` 提供，箭头与线段同源。
- 组件局部样式改为 token 引用（删除浅色字面量与浅色 fallback）：
  `brain/workbench/{style,history/style,layered/style,milestone/style,milestone/details/style,milestone/runtime/style}.css`、
  `dag/run/run.css`、`project/project.css`、`todo/review/workbench.css`、
  `ontology/pages/{directories/directories,entityTypes/details/content,graph/workspace}.css`、
  `ui/files/files.css`、`ui/runContent/style.css`。
- `crates/web/spa/src/ontology/pages/entityTypes/details/TextPreview.tsx`：HTML 预览 iframe 的 `srcDoc`
  不再是白底——文档头声明一组 iframe 内的 `--oc-*` token（`--oc-bg-layout` / `--oc-fill-subtle` /
  `--oc-border` / `--oc-border-strong` / `--oc-text` / `--oc-text-secondary` / `--oc-primary` /
  `--oc-warning` / `--oc-font-sans` / `--oc-mono`）并置 `color-scheme: dark`；body 在画布色上、`pre` /
  `code` 在浅填充 + 细边框上、`mark` 为警告色底 + 画布色字、链接为主色，`blockquote` / `table` / `hr`
  走 token，镜像 `app.css` 的深色 Markdown 面。iframe 是独立文档，外层样式表的 CSS 自定义属性不会跨文档
  边界继承，所以必须在其文档内自行声明；取值由 `theme.js` 的 `palette` / `SANS` 与 `ui/mono.js` 的
  `MONO` 派生，不另抄一份 hex。`sandbox=""` / `referrerPolicy` / CSP / `img-src data:` 与「内容只落在
  `<body>`」的行为原样保留；预览 ↔ 源码切换与非 html 走 Markdown 的分支不变。
- `crates/web/spa/src/ontology/pages/entityTypes/details/content.css`：iframe 外框
  （`.entity-html-preview`）改用 `--oc-bg-layout` 画布 + `--oc-border`，与 `srcDoc` body 同色，文档加载
  前后都不会露出白底；Markdown 预览的 `pre` / `code` 补同款 token 面；`.entity-editor-actions` 底色改为
  `--oc-panel-bg`。
- `crates/web/spa/src/agents/archiveUpload.jsx`：压缩包路径列表的内联 `#fafafa` / `#f0f0f0` 改为
  `var(--oc-fill-subtle)` / `var(--oc-border)`，合并流程与请求行为不变。

## 功能 → 测试映射

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `app.css` 先 `@import` token / shell / canvas，再是基础层 | `stylesheet wiring > app.css imports the token, shell and canvas sheets before any rule` | `crates/web/spa/src/theme.test.js` |
| 原生控件跟随深色 | `stylesheet wiring > declares a dark color-scheme so native controls follow the theme` | `crates/web/spa/src/theme.test.js` |
| CSS-only 点缀色保持；全源码无悬空 `var(--oc-*)`；`-rgb` 孪生等于 hex；浅色期字面量清零；状态底色由 `-rgb` 调配 | `CSS palette > keeps the CSS-only color %s`、`leaves no var(--oc-*) reference dangling in production sources`、`keeps every --oc-*-rgb twin equal to its hex sibling`、`keeps the light-era literals out of the stylesheets and the theme`、`mixes the %s wash from %s` | `crates/web/spa/src/theme.test.js` |
| antd 派生 token ↔ CSS 孪生逐项相等 | `palette lockstep: antd derived tokens <-> CSS twins > %s equals antd %s` | `crates/web/spa/src/theme.test.js` |
| 填充 / 三级文字 / 色阶只从 palette 派生 | `pins --oc-fill-subtle to the component tokens that consume it`、`keeps --oc-text-tertiary an opaque hex (antd dark derives rgba by default)`、`derives the washes from the palette hex, not from a second copy` | `crates/web/spa/src/theme.test.js` |
| 深色算法 + 主色钉回；色值只经 `palette`；控件边框 | `theme config shape > runs the dark algorithm and pins the reference hues after it`、`routes every palette color through the palette object`、`pins --oc-border-strong to the control border tokens` | `crates/web/spa/src/theme.test.js` |
| 圆角阶梯；48px 头部与 16px 间距；面板无阴影 / 浮层一层投影；antd 6 浮层边框 | `keeps the radius scale: Tag 4 < primary 6 (controls, cards, sheet, nodes) < floating LG 8`、`sizes the header at 48px and the content gap / padding at 16px`、`floats modal / popup / popover layers on one dark drop, panels cast none`、`borders the antd 6 panel classes, never the antd 5 element names` | `crates/web/spa/src/theme.test.js` |
| Menu pill / Table 表头 / 无浅色选中态 token | `selects the Menu pill instead of the right-hand active bar`、`drops the Table header split line and keeps rows light`、`leaves no light selected / hover / track token in the component config` | `crates/web/spa/src/theme.test.js` |
| antd code 字体与正文栈、基准字号 | `pins the antd code face to the same stack as --oc-mono`、`pins the prose face and the base size to the stylesheet` | `crates/web/spa/src/theme.test.js` |
| 等宽字体栈与 `--oc-mono` 同序、含 CJK 回退 | `mono stack > is the same stack the --oc-mono custom property declares in the token sheet`、`pins the antd code token to the same stack`、`resolves through the custom property with a bare generic fallback`、`covers the platforms the console actually runs on`、`lists the Latin monospace faces first and the Chinese fallbacks before the generic` | `crates/web/spa/src/ui/mono.test.js` |
| HTML 预览 iframe `srcDoc`：sandbox / referrer / CSP 与原样嵌入不变；从共享 token 派生深色面（非白底）；code / mark 等走 token 面；源码视图不渲染 iframe；非 html 走 Markdown | `HTML preview iframe (srcDoc) > keeps the sandbox, referrer policy and CSP while embedding the content verbatim`、`paints the dark palette from the shared theme tokens, never a light page`、`gives code and highlight elements token surfaces instead of UA defaults`、`switches to the source view: no iframe, the raw content in a pre`、`renders non-html formats as Markdown without an iframe` | `crates/web/spa/src/ontology/pages/entityTypes/details/TextPreview.test.tsx` |
| `content.css`：iframe 外框与 `srcDoc` body 同画布；无浅色字面量且每个 `var(--oc-*)` 在 tokens.css 有声明；Markdown code 面与 iframe 文档同 token | `entityTypes/details/content.css > gives the HTML preview element the same dark canvas the srcDoc body paints`、`keeps light-era literals out and every var(--oc-*) declared in tokens.css`、`styles the Markdown code surfaces with the same tokens as the iframe document` | `crates/web/spa/src/ontology/pages/entityTypes/details/content.test.js` |
| ArchiveUpload 路径列表走 token、弹窗内无浅色字面量，合并流程不变 | `paints the path list with theme tokens, never light inline literals` | `crates/web/spa/src/agents/archiveUpload.dom.test.jsx` |
| 外壳导航与布局不变 | `App shell navigation and layout > …`（含 `menu-only` 无头页逐页 `it.each`，共 20 例：执行深链优先、品牌菜单跳分类首页、chat sheet 停靠、卡片页保留 padding 等） | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 头部品牌菜单 / 分类 tabs / 页头与面板注册 / PageShell | `header brand menu > …`（8）、`keeps complete labels, scrolls overflowing tabs…`、`keeps the active category visible…`、`headerless reason registry` / `panel registry` / `every page keeps exactly one title source`（21）、`PageShell > …`（29） | `crates/web/spa/src/shell/{brandNav,categoryTabs,headerContract,pageShell}.dom.test.jsx` |
| DAG 编辑器画布（节点 / 连线 / 属性面板 / JSON 往返） | `DefEditor 画布模式 > …`、`画布连线模式 > …`、`StepInspector how_append > …`（14） | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 登录弹窗 | `LoginModal (token-only) > …`（4） | `crates/web/spa/src/login.dom.test.jsx` |
| 计划里程碑画布编辑与运行页 | `editor.dom.test.jsx`（5）、`run.dom.test.jsx`（19） | `crates/web/spa/src/brain/workbench/milestone/` |
| 内嵌 SPA 壳（编译期内嵌 `spa/dist/index.html`） | `bundle_bootstraps_the_react_root`、`static_whitelist_serves_fixed_build_outputs`、`shell_references_resolve_through_the_whitelist`、`shell_has_no_external_references`、`shell_is_the_console_entry` | `crates/web/src/html.rs` |

## 实际执行

分两轮。首轮在隔离的「纯样式」工作树完成（历史证据，见本节末）；此后无关的 SPA 改动以 HEAD `3e27f6502`
落地，共享树的 SPA 源码改动只剩本套主题，**最终交付证据以下列共享树（`/root/opencoder`）复跑为准**。

### 共享树最终结果（交付依据）

- 定向 Vitest（新增预览 / 压缩包用例落地后，显式传入 `TextPreview.test.tsx` / `content.test.js` /
  `archiveUpload.dom.test.jsx` / `theme.test.js` / `ui/mono.test.js`）→ 5 files，64 passed / 0 failed；
  同目录草稿测试 `useStructuredDrafts.test.tsx` / `useTextDrafts.test.tsx` → 2 files，7 passed / 0 failed；
  `npx tsc -p tsconfig.ontology.json` → 通过。这批在无关 SPA 提交落地之前执行，所涉源码此后未再改动
  （源码最后修改时间早于该提交与下述共享树构建）。
- `scripts/build-spa.sh` → 成功（Vite 9793 modules，仅既有的大 chunk 告警），共享树
  `spa/dist/static/app.js` 与 `app.css` 已更新为本套样式；日志 `/tmp/uitest/server-web-style/shared-final-build.log`。
- `scripts/check-spa-drift.sh` → `spa dist: no drift`；日志 `shared-final-drift.log`。
- `cargo test -p opencoder-web --lib html` → 7 passed / 0 failed（47 filtered）；日志
  `shared-final-cargo-html.log`。
- 浏览器验收（真实 Chromium，`scripts/acceptance/spa_responsive.js`，确定性夹具，宽度 1920 / 1280 / 768 /
  390）：每个宽度均访问全部 17 个注册页面，`passed: true`、`page_errors: []`、`fixture_misses: []`，每次
  布局测量 `overflow` 均为 false——1920：44 次、1280：43 次、768：43 次、390：44 次（测量次数取决于当次
  每页是否点开浮层 / 非激活 tab）。证据：`/tmp/uitest/server-web-style/shared-final-after/<width>/`
  （截图 + `receipt.json`），日志 `shared-final-after-<width>.log`。
- 专项检查（共享树 dist；`/tmp/uitest/server-web-style/shared-final-special/` 下 `receipt.json` 汇总 +
  `receipt-{1280,768,390}.json`）→ 33 / 33 通过（1280：15、768：6、390：12），25 张截图（10 / 6 / 9），
  0 页面 / 控制台错误、0 fixture miss；仅 2 条 Chrome 已知 `console.verbose` autocomplete 提示（登录弹窗）。
  覆盖：1280 登录弹窗 / 品牌菜单 / DAG 编辑器抽屉 / 执行抽屉；1280 + 768 Sider 收起 → 展开与收起后继续
  导航；390 移动端 tabs / Select / 主抽屉与 ArchiveUpload 打开 → 取消；1280 + 390 真实 Ontology 实体抽屉内
  的 HTML iframe（`color-scheme: dark`，body `#0d1117`、`pre` `#21262d`、文字 / 边框对比）与预览 ↔ 源码
  切换。验收期间 dist 五个文件的 sha256 前后一致（`dist-sha256-before.txt` ≡ `dist-sha256-after.txt`）。
- 共享树 `spa/dist`：已由上述共享树构建重新生成（相对 HEAD 仅 `static/app.js` / `static/app.css` 两个
  路径有改动），包含本套样式与预览 / 压缩包改动；漂移检查无漂移。
- Clippy：未修改 Rust，不适用。
- 测试范围：定向测试，非全量 SPA / Rust 套件；浏览器验收使用确定性 GET-only 内存夹具（非 GET 请求记为
  miss 并 405），非生产 / 真实写请求。未提交。

### 隔离树首轮（历史）

- `npx vitest run src/theme.test.js src/ui/mono.test.js src/appNavigation.dom.test.jsx src/shell/`
  → 7 files，130 passed / 0 failed（jsdom 输出已知的 canvas / 伪元素未实现告警，无失败）。
  文档阶段在共享树复跑同一命令核对 → 7 files，130 passed / 0 failed（共享树当时另含无关的在途改动）。
- `npx vitest run src/brain/workbench/milestone/editor.dom.test.jsx src/brain/workbench/milestone/run.dom.test.jsx`
  → 2 files，24 passed / 0 failed。
- `npx vitest run src/dag/editor/editor.dom.test.jsx src/login.dom.test.jsx` → 2 files，18 passed / 0 failed。
- `scripts/build-spa.sh` / `scripts/check-spa-drift.sh` / `cargo test -p opencoder-web --lib html` 在隔离树
  同样通过（`final-build.log` / `final-drift.log` / `final-cargo-html.log`：9792 modules、`no drift`、
  7 passed / 47 filtered）。
- 四宽响应式（`final-after/<width>/`，`final-after-<width>.log`）：17 页、0 溢出，测量 1920：44、
  1280：44、768：43、390：44；专项（`final-special/`）33 / 33（15 / 6 / 12）、25 张截图。
- 改动前后对照（同一脚本，`before/<width>/` 与 `after/<width>/`）：每轮每个宽度均访问全部 17 个注册页面，
  改动后每宽 44 次布局测量、0 处外层溢出（改动前 1920 为 43 次、其余 44 次），`passed: true`、
  `page_errors: []`、`fixture_misses: []`。
- 首轮专项检查（1280：登录弹窗、品牌菜单、DAG 编辑器抽屉、执行抽屉）→ `lightSurfaces: []`（无可见
  浅色面）、无外层溢出、`errors: []`、`fixtureMisses: []`（`special/`）。
- 主色统计：`/tmp/uitest/server-web-style/visual-comparison.json` 记录 Overview 等页面改动前以 `#f4f6f8` /
  `#fff` 为主、改动后以 `#0d1117` / `#161b22` 为主。

## 回滚

本改动集为单一的样式 / 测试 / 文档变更：回滚即还原上述源码、测试与文档文件，随后在共享树执行
`scripts/build-spa.sh` 重建 SPA 产物并以 `scripts/check-spa-drift.sh` 确认无漂移；无数据或 API 需要回滚。

## Impact Surface

- 全部 Web SPA 页面的颜色、字体、圆角与阴影（antd 组件与原始 CSS 面）；弹窗 / 下拉 / 抽屉 / popover 浮层
  为深色面板 + 1px 边框（浮层另有一层深色投影）。
- 原生控件（滚动条、输入控件）随 `color-scheme: dark`。
- 实体详情 HTML 预览 iframe（`srcDoc`）由白底改为深色：`color-scheme: dark`，body / `pre` / `code` / `mark` /
  链接 / `blockquote` / `table` / `hr` 全部走深色面。iframe 是独立文档，外层样式表的 CSS 自定义属性不会跨
  文档边界继承，因此在其文档头内声明同一组 `--oc-*`（取值由共享 `palette` / `SANS` / `MONO` 派生，不是复制
  的 hex）；外框与 `srcDoc` body 同为画布色。sandbox / CSP / `img-src data:` / referrer 行为不变。
- Agent 配置 ArchiveUpload 的压缩包路径列表改为深色填充 + 细边框，合并流程不变。
- 无接口 / CLI / 依赖 / Ontology 目录变更；无状态、事件、流程、路由或响应式断点变更。
