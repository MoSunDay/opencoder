# SPA 舰队控制台：IA 三分类导航 + antd 主题系统性统一

日期：2026-09-06 ｜ 模块：`crates/web/spa`（纯前端，服务端零改动）

## 动机

控制台页面数增至 12 个后，单层导航已不可扫描；各面板自带的状态
颜色/文案、时间列与页头写法逐份漂移；antd 默认样式（圆角 4、弹窗
英文按钮、红字 Text notice）与「轻量统一」的舰队定位不符。本轮把
信息架构收敛为三分类、把视觉基线收敛为一份 ThemeConfig + CSS 变量。

## 变更

### IA 三分类导航（`src/nav.js` 唯一数据源）

- `NAV_CATEGORIES`（项目 / Agent / 节点）+ `PAGE_META` + 纯函数
  `categoryOf`/`menuOf`/`categoryHome`；当前分类**纯派生**自 store
  `page`，无额外全局导航状态；`topic_detail` 折叠为所属页面
  （全部执行）的高亮。新增页面 = `items` 加一行。
- Sider 顶部 antd Segmented 三分类切换，下方 Menu 只显示当前分类
  页面（@ant-design/icons 图标组件引用存于 nav.js）；删除 Content
  顶部原桌面双 Segmented 导航；窄屏保留 Segmented+Select 下拉。
- 项目分类：项目（goal→milestone→todo 策展）、**进展**（新页：
  里程碑进度卡 / 进行中 TODO / 最近项目执行）、**Owner 视角**
  （新页：goal 健康 rollup / 待人工介入 = failed + 阻塞
  planned>24h）；Agent 分类：大脑调度、全部执行、DAG 工作流、
  TODO 管理、团队组队、会话交互、Agent 配置；节点分类：节点列表
  （原「Opencoder 列表」改名）、Env 管理。

### 主题系统（`src/theme.js` + `app.css`）

- antd v6 ThemeConfig：colorPrimary `#1677ff`、borderRadius 6、
  colorBgLayout `#f5f5f5`、Layout 白底 Header/Sider 浮于灰画布、
  Menu itemBg 透明、Table middle 密度（cell padding token）、Card
  统一 16 padding；经 `ConfigProvider theme+locale=zhCN` 生效
  （Modal 按钮中文化）。
- `app.css` 收敛为 `--oc-*` CSS 变量块，与 theme.js palette 手工
  对齐（DAG 节点等原生 CSS 面无法读 cssinjs token，两块必须同步改）。
- 全局 notice 由红字 Text 改为可关闭 antd Alert；Header 右侧 =
  连接徽标 + 服务地址 + 退出。

### 共享 UI 与数据 hook

- `src/ui/statusTag.jsx`：全控制台唯一状态→颜色/文案映射
  （吸收 fleet/model.js 执行态、todoRunsPanel 工作流/TODO 项、
  project/labels.jsx 项目态、nodes 在线态）；`fleet/model.js`
  re-export `STATUS_COLORS`/`STATUS_LABELS` 保持旧导入路径兼容。
- `src/ui/timeText.jsx`：fromNow 相对时间 + Tooltip 绝对时间
  （dayjs relativeTime 随 zhCN locale）。
- `src/shell/pageShell.jsx`：统一页头（标题/描述取自 `PAGE_META`，
  extra 动作槽，bare 跳过页头）；各面板统一换用上述三件。
- `src/project/useOverview.js`：项目概览唯一数据源（iteration 4 从
  project.jsx 行为等价抽出），`/api/project/overview` 自适应轮询——
  任一 todo running 3s、否则 8s，busy 翻转时重排 timer；项目 /
  进展 / Owner 视角三页复用同一快照与节奏，后端零改动。

## 测试清单（功能 → 测试）

| 功能 | 测试 | 层级 |
|------|------|------|
| nav 三分类数据源与分类纯派生（顺序/去重/改名/图标组件/topic_detail 折叠/menu 不泄漏他类） | `src/nav.test.js` 12 用例 | unit（纯 node） |
| 壳：Sider Segmented 切分类、删除桌面双导航、closable Alert notice、Header 退出、zhCN locale | `src/app.dom.test.jsx`（`switches categories via the Sider Segmented…`、`drops the retired desktop Segmented nav…`、`surfaces panel errors as a closable error Alert notice`、`logs out from the Header…`、`renders antd built-ins in zh-CN…` 等） | integration（jsdom） |
| 状态→颜色/文案唯一映射（含 fleet/model.js 兼容 re-export 与 unknown/missing 回退） | `src/ui/statusTag.dom.test.jsx` 7 用例 | integration（jsdom） |
| 统一页头（PAGE_META 标题/描述/extra/bare/全页面可挂载） | `src/shell/pageShell.dom.test.jsx` 5 用例 | integration（jsdom） |
| 进展页（里程碑进度 rollup / 进行中 TODO / 最近执行 / 纯函数导出） | `src/project/progressPanel.dom.test.jsx` 6 用例 | integration（jsdom） |
| Owner 视角（goal 健康 rollup / 待人工介入 failed+阻塞 planned>24h / 健康带与阈值纯函数） | `src/project/ownerView.dom.test.jsx` 8 用例 | integration（jsdom） |

## 回归证据

- SPA 全量：vitest **41 files / 412 passed / 0 failed**（本轮实跑）。
- dist 一致性：`scripts/check-spa-drift.sh` → no drift。
- 服务端零改动（纯前端轮次），无 Rust 面 diff。

## 回滚

纯 SPA 源码 + `dist` 提交物变更：还原 `crates/web/spa/src` 与
`crates/web/spa/dist` 即回滚，重新编译 server 二进制生效；无数据
迁移、无协议变更。
