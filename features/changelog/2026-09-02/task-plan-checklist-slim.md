# task-plan checklist 瘦身：移除 4-8.1 深水区清单

## 背景
内置 task-plan 的 `references/launch-closure-plan-checklist.md`（126 行）中 4-8.1 节（功能与兼容性闭环 / 数据配置与安全 / 验证与回归闭环 / 线上或生产等价验证方案 / 发布准备与上线后保护 / 持续保鲜与稳定性）把每次规划都拖向上线窗口级深水清单——SKILL.md 五要素已覆盖验证手段与证据成熟度，深度细则属过度披露。按要求 4、5、6、7、8（含 8.1）完整去掉。

## 变更
- **`crates/core/assets/skills/task-plan/references/launch-closure-plan-checklist.md`**：删除 `## 4.` 到 `## 8.1` 全部小节，126 行 → 61 行；保留 1 需求与现状审查、2 根因与缺口识别、3 代码与模块影响、9 遗漏复查与交付可读性、Plan Output Schema。
- **`crates/core/assets/skills/task-plan/SKILL.md`**：checklist 引用括号由「合约、保鲜与发布细则见」改为「审查与验收细则见」。
- **`crates/core/tests/skill_contract.rs::seeded_task_plan_skill_requires_launch_closure_contract`**：「持续保鲜与稳定性」从存在断言改为**负向锁**（不得再 seed 回来），并新增 4 个保留节的存在断言。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 4-8.1 不回归 + 保留节锁定 | `seeded_task_plan_skill_requires_launch_closure_contract` | crates/core/tests/skill_contract.rs |
| SKILL.md 用法契约 | `seeded_task_plan_skill_requires_question_tool_guidance` | crates/core/tests/skill_contract.rs |
