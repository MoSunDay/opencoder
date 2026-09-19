# 调度定义迁入库：schedules 表（schema v27）成事实源，admin CRUD + 手动触发 + SPA 全功能管理

## 背景

- 调度定义一直只存 `schedules.json`（控制面工作目录的领域文件）：改定义
  要登机改文件，无法从 Web/API 侧新增、启停或删除；页面是只读列表。
- 台账（`schedule_runs`，schema v26）早已入库，定义却游离在文件里，两半
  割裂；多副本/迁移场景下文件同步也是隐性负担。

## 变更

- **store（schema v27）**：新增 `schedules` 定义表（主键 `id`、`job` JSON、
  created_at/updated_at；bootstrap 建 v1 即含，`if from < 27` 增量迁移）。
  `Store` trait 新增 `upsert_schedule/get_schedule/list_schedules/
  delete_schedule` 四方法；`ScheduleDefRecord`（job JSON + 时间戳）入
  `schedule_types`。upsert 冲突保留 created_at、更新 updated_at；删除
  定义不级联台账（无 FK，审计长存）。
- **control**：
  - 调度器 `scan()` 改读 `store.list_schedules()`（读失败仅延迟本扫描）；
    `fire_tick` 返回 `Result<String>` 透传派发失败；新增 `fire_now`。
  - `/api/schedules` admin CRUD：`POST` 创建（缺省 id 自动 `schedule-<ULID>`、
    重名 409、非法 body 400）、`PUT /:id` 全量更新（404 未知 id、created_at
    保留）、`PATCH /:id` 仅启停（重校验整个定义，坏 cron 的停用条目启用
    即 400）、`DELETE /:id`（历史保留）、`POST /:id/run` 手动立即触发
    （绕过 enabled/overlap，显式操作员动作，台账照记）。`ScheduleJob::validate`
    是唯一校验门，路由仍走 role_gate 默认关（admin-only）。
  - `seed_schedules.rs`：遗留 `schedules.json` 一次性导入（仅表空才执行，
    skip-don't-merge——删除不会在重启时复活；非法条目 warn 跳过不阻断
    启动）；`scan_interval_secs` 保留文件职责，调度循环热读不变。
- **SPA**：「定时任务」页从只读列表升级为全功能管理——工具栏「新建任务」，
  行操作 触发历史 / 立即触发（Popconfirm）/ 编辑 / 启停 / 删除
  （Popconfirm，提示历史保留）；新增 `schedule/editor.jsx` Modal 表单
  （id 编辑态锁定、cron、时区、kind、target、params JSON 校验、overlap、
  节点选择复用 `nodeOptions`、启用开关；DAG 目标禁用 params）；info 提示
  改为「存于控制面数据库（schedules.json 仅作首次导入种子）」。
- 文档同步：`docs/agent-platform.md`、`features/agent-platform/index.md`、
  `agents/control/index.md`、`agents/store/index.md`。

## 测试

- 新增 `crates/store/tests/schedule_defs.rs` 5 例：定义 roundtrip、
  created_at 保留/updated_at 前移、稳定 id 序、删除定义后台账仍查得到、
  bootstrap 关库重开幂等。
- 迁移版本钉 26→27：`store_migrations/{early,middle,catalog,sessions,
  project_replay}`、`brain_store`、`display_text`。
- `crates/control/tests/e2e/schedule_api.rs` 重写为库定义口径，11 例：
  列表（含停用坏 cron 条目 next_run=null fail-soft）、CRUD 全往返
  （409/400/404/auto-id/created_at 保留/停用坏 cron 不可启用）、API 建
  条目真实触发 + PATCH 停用后静默、DELETE 停止触发且历史保留、手动触发
  （绕过 enabled，台账 + last_run）、`overlap: skip` 终态门、24h 追赶
  `missed` 折叠、文件 seed 一次性（表空才导入、改文件不回灌、非法条目
  跳过）、brain 缺 objective 400 且健康兄弟不受饿、admin-only 五写路
  403、非法 id 400。
- SPA：`schedule/panel.dom.test.jsx` 9 例（列表契约、空态、错误通知、
  新建 POST、编辑 PUT 保 id、PATCH 启停 + Popconfirm 删除、手动触发、
  保存失败通知、触发历史 Drawer 三态）；全量 `npx vitest run` 842/843
  （1 例 `admin/usersDrawer` 首跑 5s 超时为既有负载抖动，单跑通过）。
- 回归：`cargo test -p opencoder-store --tests` 全绿（含迁移套件）；
  `cargo test -p opencoder-control --test e2e schedule_` 11 通过。
- dist 已重建（`crates/web/spa/dist/static/app.js` 内嵌二进制随之更新）。
