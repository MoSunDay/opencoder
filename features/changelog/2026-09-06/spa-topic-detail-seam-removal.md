# SPA topic_detail 深链死缝整体移除（含 teamItems 遗留死导出清理）

日期：2026-09-06 ｜ 模块：`crates/web/spa`（纯前端，服务端零改动）

## 动机

上轮评审（spa-store-dead-seam-cleanup）问五 #1：`openTopicDetail` 全仓零
调用者，topic_detail 深链页仅测试直驱 `setState` 可达，建议下轮「接通或
删除」。本轮取证后裁决为**删除**——「接通」在域上不成立：

- `topicDetail.jsx` 拉取 `/api/teams/:name/topics/:tid`（团队话题域：轮次
  时间线/汇报计划/成员汇报/对齐链），而 `topics` 页 IA 重构（4519f75）后
  已是 `fleet/executions.jsx` 平台执行列表，自带 `fleet/detail.jsx`
  ExecutionDetail 抽屉——两域对象不同，无可挂接的行；
- 组件返回按钮文案「← 返回话题列表」指向的页面形态已不存在；
- nav.js:78「执行详情」标题是 IA 重构期的文案挪用，折叠目标 `topics`
  语义已失配。产品层无裁决输入（触发条件未满足），按死缝清理轨迹与
  域错配实证选删除路径。

## 变更

- 删 `src/topicDetail.jsx`（223 行整文件）。
- `store.js`（120→107）：删 `topicDetail` 字段（初始态 + 两处凭证清理
  重置）与 `openTopicDetail`/`closeTopicDetail` 导出；page 注释枚举收缩。
- `main.jsx`（206→198）：删 import 与 PANELS `topic_detail` 项；
  `goPage('topics')` 特例收敛为普通 `setState({ page: key })`——直达重置
  语义随「无残余可重置状态」自然消解（上轮保留的重置行为已无对象）。
- `nav.js`（135→127）：删 PAGE_META `topic_detail` 项；`categoryOf`/
  `menuKey` 去折叠分支，注释同步改真。
- 测试同步：`team.dom.test.jsx`（306→279，删 TopicDetailPanel describe
  2 例 + beforeEach 字段）、`nav.test.js`（153→148，删/改 3 处折叠断言；
  PAGE_META 覆盖测试改为与 ALL_PAGES 严格相等，反向看住未来死条目）。
- **范围扩展（超出上轮清单，repair-on-touch）**：`teamItems.js`（159→47）
  + `teamItems.test.js`（174→50）——11 个导出逐一 grep 验证零非测试消费
  者后删除：6 个仅被 topicDetail.jsx 消费（fmtTime 转出口、
  topicStatusView、turnAligned、subTurnCount、resultLabel、ambiguityText），
  5 个为 IA 重构遗留的先存死导出（finishReasonText、teamCapSummary、
  turnTimelineItems、topicCancelable、topicResumable），连同仅供其使用的
  FINISH_VIEW 等模块常量一并清；保留 teamModals.jsx 活消费的
  captainOptions/memberCapsText/nodeSelectOptions。

## 测试清单（功能 → 测试）

| 功能 | 测试 | 层级 |
|------|------|------|
| 删除后 fleet 真源面板行为不变（团队/执行/取消恢复全链） | `src/team.dom.test.jsx`（TopicDetailPanel describe 移除，余 7 用例全绿） | integration（jsdom） |
| nav 单一真源：无折叠后 categoryOf/menuKey 回退与 PAGE_META 严格覆盖 | `src/nav.test.js`（menuKey fallbacks / covers every page key exactly） | unit（纯 node） |
| team modal 活契约（能力摘要/节点/队长选项）不受牵连 | `src/teamItems.test.js` 保留 3 describe | unit（纯 node） |

## 回归证据

- SPA 全量：vitest **42 files / 402 passed / 0 failed**（build 子代理跑 ×1
  + 父代理独立复跑 ×1 双绿；417→402 的 −15 与删除的死功能用例数吻合）。
- 删除完备性：`grep -rn 'topicDetail|topic_detail|TopicDetail' src dist`
  → 零命中（含重编后 bundle）。
- dist 一致性：`scripts/build-spa.sh` 重编 + `scripts/check-spa-drift.sh`
  → no drift（仅 `dist/static/app.js` 变更；父代理门禁期二次复核）。
- Rust 面：`cargo build --workspace` ok、`clippy --workspace --all-targets
  -D warnings` 零警告、`cargo test --workspace`（与 SPA 门禁严格串行执行）
  → **4752 passed / 0 failed**（5 ignored 为存量，与上轮基线持平）。
- 行数：store.js 107 / main.jsx 198 / nav.js 127 / teamItems.js 47 /
  team.dom.test.jsx 279，全 < 800。

## 上轮问五处置记档

1. topic_detail 接缝：本轮删除（依据见动机节）。
2. CI 门禁串行化：触发条件不可满足——仓库唯一 workflow
   `.github/workflows/project-sql-tests.yml` 仅跑 store/project Rust 契约
   测试，SPA 套件**不在任何 CI 中**，无 runner 核数可查；本地纪律（SPA 与
   cargo 门禁串行）沿用上轮记档，本轮已实际执行。
3. `server-e2e-full-coverage.md` 历史归属瑕疵：维持留观（内容已由 b70be00
   修正），搭下次 docs 批次顺手说明。
4. PageBody 未知页不对称：前已存在，仍留独立迭代。

## 回滚

纯 SPA 源码 + `dist` 提交物变更：还原 `crates/web/spa/src` 与
`crates/web/spa/dist` 即回滚，重新编译 server 二进制生效；无数据迁移、
无协议变更、后端 `/api/teams/:name/topics/:tid` 端点未动。
