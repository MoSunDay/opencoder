# SPA store 话题过滤死缝清理 + notice 白名单断言补齐

日期：2026-09-06 ｜ 模块：`crates/web/spa`（纯前端，服务端零改动）

## 动机

上轮评审（spa-notice-type-and-dead-code-cleanup）静态核查新发现的死缝：
IA 三分类重构后 fleet/executions 改用 `kind=team` Select 过滤，
`store.js` 的 `openTopicsForTeam` / `setTopicsTeamFilter` 全仓零调用者，
`topicsTeamFilter` 字段沦为只写状态（初始态/clearCredentials/clearToken/
goPage 四处重置、无任何读取方）。同轮补齐评审 nit：
`normalizeNotice` 的 type 白名单分支缺直接断言。

## 变更

- `store.js`（133→120 行）：删 `openTopicsForTeam` / `setTopicsTeamFilter`
  两个导出及 `topicsTeamFilter` 字段（初始态 + 两处凭证清理重置同步收缩）；
  `closeTopicDetail` 注释随之改写（team filter 已不存在）。
- `main.jsx`：`goPage('topics')` 直达重置由三字段缩为
  `{ page, topicDetail }` 单字段——「直达落干净视图」的重置语义本身保留
  （topic_detail 深链参数仍被丢弃），仅团队过滤维度随字段一并消失。
- `team.dom.test.jsx` beforeEach 状态重置同步去字段。
- `notice.test.js`：illegal-inputs 用例补一行
  `normalizeNotice({ type: '非法值', text: … })` 白名单兜底断言
  （缺 type / 非法 text 此前已覆盖，本轮补 type 越界值）。
- 范围澄清：`openTopicDetail`/`closeTopicDetail`/`topicDetail` 为
  topic_detail 深链页（nav.js 折叠、team.dom.test 保留 describe）仍依赖的
  存量接缝，本不动；`openTopicDetail` 当前亦无 UI 调用方，列为下轮观察。

## 测试清单（功能 → 测试）

| 功能 | 测试 | 层级 |
|------|------|------|
| 死缝删除后 store 状态面不变（fleet 真源团队/执行/取消恢复/topic_detail 回退） | `src/team.dom.test.jsx` 9 用例（含 `goPage` 重置语义对齐的 beforeEach 同步） | integration（jsdom） |
| normalizeNotice type 白名单越界值兜底 | `src/notice.test.js` `falls back to an empty error notice for illegal inputs` | unit（纯 node） |

## 回归证据

- SPA 全量：vitest **42 files / 417 passed / 0 failed**（独立复跑 ×3 全绿）。
  诚实记录：首跑与 `cargo test --workspace` 并发执行时出现 1 例超时
  fail（416/417），CPU 争用所致——直接实证上轮观察项「vitest 池上限按
  16 核调，核数不足/争用环境需随行验证」，Gate 实跑应避免与 cargo 并发。
- dist 一致性：`scripts/build-spa.sh` 重编 + `scripts/check-spa-drift.sh`
  → no drift（仅 `dist/static/app.js` 变更）。
- Rust 面零改动；同轮全量 `cargo build --workspace` /
  `clippy --workspace --all-targets -D warnings`（零警告）/
  `cargo test --workspace` → **4752 passed / 0 failed**（5 ignored 为存量）。
- 行数：store.js 120 / main.jsx 206 / team.dom.test.jsx 306 / notice.test.js 38，全过。

## 遗留观察（不阻塞）

- `server-e2e-full-coverage.md` 随 SPA 线 commit 入库的历史归属瑕疵：内容
  已被 b70be00 修正，留观。
- PageBody 未知页「亮 nodes 菜单却渲染 ChatPanel」不对称为前已存在，未处理。

## 回滚

纯 SPA 源码 + `dist` 提交物变更：还原 `crates/web/spa/src` 与
`crates/web/spa/dist` 即回滚，重新编译 server 二进制生效；无数据迁移、
无协议变更。
