Commit: (working-tree)

# SPA「定时任务」页：cron 调度控制台入口

> 后端（`272b2b13`：`GET /api/schedules`、`/api/schedules/:id/runs`、CLI
> `schedule list|runs`）已就绪但 SPA 零实现；本迭代补 Agent 分类下的独立
> 调度菜单页，前端至此与查询面对齐。

## 变更

- 导航：Agent 分类「全部执行」之后新增 page `schedules`（menu「定时任务」，
  `ClockCircleOutlined`），`HEADERLESS_REASONS.schedules = 'menu-only'`（操作型
  列表页，与 topics/nodes 同口径）。菜单文案不用裸「调度」：app.dom.test 的
  menuitem 名字按 `/label$/` 锚定匹配，「调度」会被「大脑调度」后缀误命中。
- `spa/src/schedule/panel.jsx`：`GET /api/schedules` 只读 Table（id/cron/
  enabled(Tag)/kind/target/overlap/node_id/last_run(TimeText+StatusTag)/
  next_run；invalid cron 的 next_run 为 null 渲染「—」）；页顶 Alert 声明
  schedules.json 是唯一定义事实源 + `scan_interval_secs`；手动刷新 + 5s 静默
  轮询。操作列「触发历史」→ Drawer 调 `GET /api/schedules/:id/runs?limit=50`，
  展示 scheduled_for_ms/fired_at_ms/status(fired/missed/error)/execution_id/
  error。不做编辑（与后端契约一致）；非 admin 本就不可见（`allowedPages`）。
- `ui/statusTag.jsx`：STATUS_META 吸收调度台账状态 `fired`（已触发/success）、
  `missed`（已错过/default）；`error` 复用既有「失败」行。
- `shell/panels.jsx`：`PANELS.schedules = SchedulePanel`。
- 顺手清理两处 HEAD 上既有的 clippy 死代码（`-D warnings` gate 被它们卡死）：
  `store/tests/delete_sessions_and_indexes.rs` 未用 import、
  `worker/tests/maintenance_dialogs_clear.rs` 未用 import + 未用 `done` fn。
- dist 重建并提交（`scripts/build-spa.sh`，`check-spa-drift.sh` 零漂移）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 定义列表 + last/next_run + null「—」 | `lists the schedules.json definitions with last/next fire` | `crates/web/spa/src/schedule/panel.dom.test.jsx` |
| 空态 + 无 scan_interval_secs 提示 | `shows the empty state when schedules.json declares nothing` | 同上 |
| 失败经 onNotice(err) 透出 | `surfaces a failed list load through onNotice(err)` | 同上 |
| 历史 Drawer 三种台账 status | `opens the fire-history drawer with all three ledger statuses` | 同上 |
| fired/missed 状态文案与颜色 | `renders the schedule ledger statuses (fired / missed)` | `crates/web/spa/src/ui/statusTag.dom.test.jsx` |
| 导航注册（顺序/menu-only/面板映射） | `lists category pages in menu order` 等 | `crates/web/spa/src/nav.test.js`、`shell/headerContract.dom.test.jsx`、`app.dom.test.jsx`（menu-only 移动端 Select 循环自动覆盖新页） |

- SPA：`npm run test` → 838 passed / 0 failed（114 文件）
- 全量回归：`cargo test --workspace` → 见下方执行记录
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
