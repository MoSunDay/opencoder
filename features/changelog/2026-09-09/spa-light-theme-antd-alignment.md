Commit: (working-tree, 基于 e488f767)

# fleet console 浅色主题精修：调色板锁步守卫 + antd 6 最佳实践对齐

继 e488f767（冷中性调色板重调 + CSS 打磨 + 锁步守卫）之后，完成同一轮精修的后两段：把散落在各面板的**内联硬编码色值**与**五种等宽字体拼写**收敛到单一事实源，清理 antd 6 的**废弃 API**（`Space direction` / `Alert message` / `Descriptions.Item` / `Timeline items.children` / `Drawer width`），把 22 处**静态 `message.*`** 迁到 App 上下文，并补齐 5 张表格的 `loading` 反馈。

新增三个纯数据/纯函数原语，避免各处重复字面量：

- `src/ui/mono.js` — `MONO` 与 `MONO_VAR`（`'var(--oc-mono, monospace)'`）；`theme.js` 的 `--oc-mono` 现直接取自 `MONO`，`mono.test.js` 断言两者恒等。
- `src/ui/monoText.jsx` — `<MonoText>`，供需要语义包装的调用点使用。
- `src/ui/appMessage.js` — `useMessage()`：在 antd `<App>` 内返回上下文 message API，**否则回落到静态 API**。回落分支是必需的：antd 的 AppContext 默认值是 `{ message: {} }`，裸调 `App.useApp().message.success()` 会直接抛错，因此 DOM 测试单独挂载某个面板时不能崩。

调色板侧新增 `--oc-accent-user` / `--oc-accent-ai` 及其 `-rgb` 孪生与 `--oc-heading`（`theme.js cssVars` 与 `app.css :root` 同步，仍受 e488f767 的锁步守卫覆盖）。`-rgb` 孪生存在的原因：`transcript.jsx` 的 RoleAvatar 原先用 `color + '1a'` 字符串拼接构造 10% 淡底，这种写法无法用 `var()` 表达，改为 `rgba(var(--oc-accent-user-rgb), 0.1)`（0x1a/255 ≈ 0.102，等价）。另加 `.oc-row-selected` 取代借用 antd 内部 `.ant-table-row-selected` 伪造选中行的做法。

刻意**未**收敛的三处，避免改动用户可见文案：`dag/runDetail.jsx` 的 `STEP_LABEL`（DAG 步骤专用词表）、`transcript.jsx` 的局部 `StatusTag`（`chat.dom.test.jsx` 锁定字面量 `streaming…`）、`fleet/detail/workloads.jsx` 的复合 ID 胶囊（`<Tag>{id} · {status}</Tag>`，转成 StatusTag 会拆成两个元素并改文案）。`subagentBlock.jsx` 的私有 `STATUS_COLOR` 已删除并委托给共享 `statusColor`，但保留 `statusColorOf` 导出与四键白名单，使未知状态仍回落 `'processing'`（共享实现对未知态返回 `'default'`，不加白名单会静默改变行为）。`transcript.jsx` 的 `marginTop: 16` 与 `stepsBlock.jsx` 的 `marginLeft: 12` 是 `scripts/browser-acceptance-saypairs.js` 的定位锚点，原值未动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 等宽栈 JS/CSS 单一事实源 | `is the same stack the --oc-mono custom property declares` | `crates/web/spa/src/ui/mono.test.js` |
| 等宽栈保留裸通用族回落 | `resolves through the custom property with a bare generic fallback` | `crates/web/spa/src/ui/mono.test.js` |
| 等宽栈覆盖实际控制台平台 | `covers the platforms the console actually runs on` | `crates/web/spa/src/ui/mono.test.js` |
| 新增 accent/heading 变量两侧同名同值 | `declares exactly the same set of --oc-* variables` / `gives every shared variable the same value` | `crates/web/spa/src/theme.test.js` |
| `-rgb` 孪生由 hex 推导，不得漂移 | `derives --oc-primary-rgb from --oc-primary` | `crates/web/spa/src/theme.test.js` |
| 无悬空 `var(--oc-*)` 引用 | `leaves no var(--oc-*) reference dangling on a fallback` | `crates/web/spa/src/theme.test.js` |
| antd token ↔ CSS 孪生 14 项 | `--oc-primary equals antd colorPrimary` 等 | `crates/web/spa/src/theme.test.js` |
| 半径阶梯 4 < 6 < 8 < 10 / Menu 胶囊 / Table 无分割线 | `keeps the deliberate radius scale…` 等 | `crates/web/spa/src/theme.test.js` |
| 废弃 API 清零 + 无 console 告警门禁 | `app.dom.test.jsx` deprecation gate | `crates/web/spa/src/app.dom.test.jsx` |
| Say/Step 阶梯定位锚点与流式状态文案 | `pairs separate persisted messages…` / `chat.dom.test.jsx:39` | `crates/web/spa/src/saypairs.e2e.test.js`, `src/chat.dom.test.jsx` |
| `statusColorOf('done') === 'success'` 等四态 | `subagentBlock.dom.test.jsx:100` | `crates/web/spa/src/subagentBlock.dom.test.jsx` |

## 验证结果

- 守卫用例：`npx vitest run src/theme.test.js src/ui/mono.test.js` → **26 passed**（e488f767 的 23 + 本轮 mono 3）。
- SPA 全量：`npx vitest run` → **67 files / 541 tests passed**，本轮改动文件零 `deprecated` 告警。
- 分组定向：`src/dag/` 48 passed；transcript/chat 组 7 files / 58 passed；todo/project/fleet 组 16 files / 98 passed；agents/envs/harness/team 组 10 files / 38 passed。
- 清理证明 grep（改动文件范围）全部为 0：`fontFamily: 'monospace'`、裸 `'var(--oc-mono, monospace)'` 字面量、`#[0-9a-fA-F]{3,6}`、`Space direction=`、`Alert message=`、`Descriptions.Item`、`Timeline items.children`、`ant-table-row-selected`、`<a onClick`、静态 `message.(success|error|…)`。
- 行数 gate：本轮改动文件均 ≤ 508 行（上限 800），新增文件 ≤ 60 行（上限 400）；无新增依赖、无动态 `import()`、无 class、无硬编码凭据。

## 并行会话在途状态（未触碰）

工作树中存在另一路并行会话的 SPA 在途改动：`src/main.jsx`（权限化 IA：`nav.js` API 重命名 + `UsersDrawer` + `IdentityBadge`）、`src/agentsConfig.jsx`（admin-only `OperatorPanel` 页签）、`src/store.js` / `src/nav.js` / `src/login.jsx` / `src/fleet/detail.jsx` / `src/fleet/model.js`，以及未跟踪的 `src/admin/`、`src/operators/`。因此本次提交**刻意排除** `src/main.jsx`、`src/agentsConfig.jsx` 与 `dist/`：

- `main.jsx` 中包裹 shell 的 antd `<App component={false}>` 与其 `UsersDrawer` 挂载在 JSX 上相互嵌套，无法干净拆分；提交它会连带引入对未跟踪 `src/admin/usersDrawer.jsx` 的 import，使该提交单独不可构建。`useMessage()` 的静态回落分支保证在 `<App>` 落地前行为与迁移前完全一致（无回归），`<App>` 一落地即自动升级为上下文 API。
- `dist/` 未重建提交：当前重建会把上述未提交的源码烘进已提交产物，违反 dist↔src 契约；且并行会话自身的提交（`708220d7`、`406d1ffd`）同样未带 dist。`scripts/check-spa-drift.sh` 目前为红，归因于该在途改动，待其落地后由重建统一收敛（届时会一并包含本轮 src 改动）。

## 后续修正（见 `spa-table-loading-honesty-and-guard-hardening.md`）

本文两处表述经评审核实**不成立**，特此更正，不改写原始记录：

- 「`mono.test.js` 断言两者恒等」当时是恒真断言：`theme.js` 里就是 `'--oc-mono': MONO`，
  该测试实为 `MONO === MONO`，不构成 JS↔CSS 守卫（真正生效的是 `theme.test.js` 的动态枚举比较）。
  现已改为直读 `app.css` 的 `:root` 声明。
- `src/ui/monoText.jsx` 作为「交付原语」列出，但全树零 importer、自身无测试，属死代码，现已删除；
  实际被约 32 处调用点使用的原语是 `ui/mono.js` 的 `MONO_VAR`。
