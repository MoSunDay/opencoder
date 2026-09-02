Commit: 51b2dbcf2c4d63b147aa0a2bdb29fb91fc9ef227 (checklist 残留节重排)

# task-plan checklist 残留节重排：## 9. → ## 4. 序号连续

## 背景

`e8f029c`（checklist 瘦身）删除 4-8.1 全部小节后，残留的「遗漏复查与交付可读性」仍挂 `## 9.` 序号，保留节呈 1、2、3、9 断层。序号漂移同时传染已 seed 的机器：seeding 对已存在文件 never-clobber，旧二进制不会自动把 seeded 副本收敛到重排后的 ship 版。

## 变更

- `crates/core/assets/skills/task-plan/references/launch-closure-plan-checklist.md`：`## 9. 遗漏复查与交付可读性` → `## 4. 遗漏复查与交付可读性`，保留节 1-4 序号连续。
- `crates/core/tests/skill_contract.rs::seeded_task_plan_skill_requires_launch_closure_contract`：保留节断言升级为**带序号标题**（`## 1.`~`## 4.` + `## Plan Output Schema`），序号断层回归即红。
- 已 seed 机器的收敛路径：同日起内置包 seeding 改为 update-on-drift（漂移文件自动备份并覆盖为 ship 版，见同日 review 回退条目），漂移的 seeded 副本由下一次启动自动收敛，无需手动删除。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 保留节序号连续锁定 | `seeded_task_plan_skill_requires_launch_closure_contract` | crates/core/tests/skill_contract.rs |
