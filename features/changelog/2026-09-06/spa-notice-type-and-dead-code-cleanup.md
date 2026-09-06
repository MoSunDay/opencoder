# SPA 全局通知成功/错误类型区分（评审 R1 修复）+ 遗留死代码清理

日期：2026-09-06 ｜ 模块：`crates/web/spa`（纯前端，服务端零改动）

## 动机

评审缺陷 R1：壳层 Alert 恒 `type="error"`，goalsTab/todosTab/todoDrawer
等 19 处成功文案（「目标已创建」「草稿已保存」「里程碑已更新」…）以
红色错误样式呈现，成功/错误不可区分。同时 IA 三分类重构后，顶层
`nodes.jsx`/`teamPanel.jsx`/`topicsPanel.jsx` 三个旧面板副本（共 513
行）已被 `fleet/` 目录取代（`main.jsx` 直连 `fleet/nodes|teams|executions.jsx`），
仅 `team.dom.test.jsx` 仍引用，属死代码。

## 变更

### 通知载荷契约（`src/notice.js`，新 34 行）

- 纯函数构造器 `ok/err/info/warn` → `{type, text}`；`normalizeNotice`
  全定义域安全：裸字符串按 err（旧调用点兼容）、非法输入兜底
  `{type:'error', text:''}`。
- `main.jsx`：notice 态改对象；Alert `type={notice.type}` 派生渲染、
  `title={notice.text}`；`onNotice` 经 `useCallback` 固定标识（面板普遍
  以 `[onNotice]` 作 load effect 依赖，内联箭头每次渲染换标识会引发
  无限重取——实测挂死测试套件）；空文本（`err('')` 清屏惯例，11 处）
  不渲染；关闭/登录成功重置 null。
- 全量 125 处调用点分类包裹（**文案逐字保留**）：err 95 / ok 19 /
  info 5（异步已受理与指引）/ warn 6（前置校验）。

### 死代码清理

- 删除 `src/nodes.jsx`(142 行)/`src/teamPanel.jsx`(210)/
  `src/topicsPanel.jsx`(161)；收尾 grep 引用清零。
- `team.dom.test.jsx` import 重定向 fleet 真源，9 用例断言对齐 fleet
  行为（STATUS_META 状态 Tag、`kind=team` 过滤、drawer 取消/恢复、
  `/api/agents` 喂养的成员 picker），**用例数不减、语义不削弱**；
  「查看话题 arms topics tab」等价迁移为「启动团队 arms launch modal」
  （候选断言搬到其真实存在处）。`teamItems.test.js` 头注释同步。
- `agents/web/index.md` 组队面一句更新（fleet 真源表述）+ notice 载荷
  契约一句补正。

### 顺手

- `progressPanel.jsx` 执行列表兜底 `(page && page.executions) || []`
  → `page.executions || []`，与 `fleet/executions.jsx` 同口径。

## 测试清单（功能 → 测试）

| 功能 | 测试 | 层级 |
|------|------|------|
| notice 载荷构造器 + normalizeNotice 全定义域（字符串/对象/null/缺字段） | `src/notice.test.js` 4 用例 | unit（纯 node） |
| 壳双断言：错误流（failNodes → `ant-alert-error` + closable）与成功流（App 级新建目标 → `ant-alert-success` 且含「目标已创建」，fetch router 最小扩展 `withGoal`） | `src/app.dom.test.jsx`（既有 error 用例补类型断言 + 新增成功流用例） | integration（jsdom） |
| 面板通知断言对象化（err/info 载荷穿透） | `src/todoPanel.dom.test.jsx`、`src/fleet/fleet.dom.test.jsx`、`src/dag/dag.dom.test.jsx` | integration（jsdom） |
| 死代码删除后 fleet 真源行为不回退（团队行/创建团队+成员 picker/启动团队/执行过滤/取消恢复） | `src/team.dom.test.jsx` 9 用例 + `src/teamItems.test.js` 16 用例 | integration + unit |

## 回归证据

- SPA 全量：vitest **42 files / 417 passed / 0 failed**（本轮实跑；
  412→417 = notice 4 + 成功流 1）。
- dist 一致性：`scripts/check-spa-drift.sh` → no drift。
- Rust 面零改动；同轮全量 `cargo build`/`clippy --all-targets`/`test
  --workspace` 附跑全绿（见同日提交）。

## 回滚

纯 SPA 源码 + `dist` 提交物变更：还原 `crates/web/spa/src` 与
`crates/web/spa/dist` 即回滚，重新编译 server 二进制生效；无数据
迁移、无协议变更。
