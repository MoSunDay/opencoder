Commit: df42d352eedc014416b8a37c743e49b6a68aeeb4

# 控制台导航选择（项目 / Agent / 节点）localStorage 持久化

控制台左侧三分类 Segmented（项目 / Agent / 节点）与分类内页面的最后选择跨刷新保留：显式导航（Sider Segmented/Menu、移动端 Segmented/Select，统一汇入 `goPage`）经 `usehooks-ts` 的 `useLocalStorage` 把所选页写入 localStorage（`oc_nav_page`），重新挂载时在首帧绘制前（`useLayoutEffect`）恢复进 store，默认页不闪现。SPA 源码零手写 `getItem/setItem`，localStorage 读写全部在 hook 库内。

## 变更

- `crates/web/spa/package.json`：新增 `usehooks-ts@3.1.1`（React 最佳实践 hook 库，peer React 16.8–19）。
- `crates/web/spa/src/nav.js`：新增 `NAV_STORAGE_KEY = 'oc_nav_page'` 与派生的 `ALL_PAGES` 页面校验集（单源 NAV_CATEGORIES，不可漂移）。
- `crates/web/spa/src/main.jsx`：App 持有 `useLocalStorage(NAV_STORAGE_KEY, null)`；`goPage` 显式导航即镜像；`useLayoutEffect` 首帧前恢复（`brain_run` 深链优先、`ALL_PAGES` 外的陌生/损坏值忽略回退默认页）。
- `crates/web/spa/dist/`：`npm run build` 产物刷新（`html.rs` 编译期内嵌，无哈希钉扎测试）。

## 边界

- 分类仍纯派生自 store `page`，无第二份运行时导航状态；非 admin 身份沿用既有 `shownPage` 钳制。
- 程序化跳转（`store.openChatForNode`，当前无生产调用方）保持仅写 store 的既有语义，不镜像。

## 验证

- `crates/web/spa` vitest 全量：110 文件 / 803 用例通过；新增 `src/navPersistence.dom.test.jsx` 4 例（显式导航写入与 JSON 形状、重挂载恢复菜单高亮、陌生/损坏值回退、brain_run 深链优先）。

## 发布：rel-653c7162 本机平滑上线（同日 09:19）

按「最新逻辑生效」要求，将 HEAD（`653c7162`，含本特性、大脑发起表单 KV 化、Operator 入口更名）构建为发布包并本机平滑发布，无停机：

- 构建前置：SPA 全量回归 803 passed（`653c7162` 干净树上复跑）；`scripts/platform/release/build.sh --output /srv/releases/opencoder-653c7162` 通过 SPA 漂移检查与编译期 SPA 摘要核验（manifest `spa_sha256 5f8aacce…`，协议 10）。
- 发布：`scripts/platform/deploy.sh --bundle /srv/releases/opencoder-653c7162 --wait-seconds 300`（普通平滑通道，候选预热 + WASM 探针 + nginx reload），phase complete，current 切至 `rel-653c7162e3ec8efb43a7a3cf2e7f249551df071d`，旧 rel-2dc1323d 按既有执行进入退役回收。
- 线上核验：`opencoder-server 0.1.0 (653c7162) listening on http://127.0.0.1:3045`；nginx 公共入口 `/static/app.js` 含 `oc_nav_page` 持久化逻辑（HTTP 200），三件套 systemd unit active。
