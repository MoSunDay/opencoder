# task-plan skill 瘦身为五要素输出契约

## 背景

`task-plan/SKILL.md` 已膨胀到 17.9KB / 213 行：核心流程列表、Workflow 八节、
Final Output 模板与 `references/launch-closure-plan-checklist.md` 大面积互相
重复（合约/保鲜、计划项字段、验证参数各出现两到三次），激活后浪费大量上下文。
本次仅做内容瘦身，**不改变交付方式**：仍是手动 `$task-plan` 激活的一次性
skill，`question` 工具及其用法仍内置于本 skill 的澄清协议，不从内置包剔除。

## 变更摘要

- SKILL.md 重写为 33 行 / ~4KB（-78%），输出契约收敛为五要素：
  `树立目标`（含 gating item）、`关键 context`（约束 + 事件流程回看 + 现状
  三分审查 + 影响面范围纪律）、`TODO List`（P0-P3 + 动作/完成定义/依赖/状态，
  复杂需求再细）、`TODO 验证手段`（实质校验 + 线上/生产等价验证 + 证据成熟度
  五级不跨级）、`核心动作`（关键路径：上线前/并行/窗口内/观察回滚）。
- 澄清协议完整保留：先查再问、`question` 用法（一次一问、≤4 选项、同轮可
  多问）、不可用时 `assumptions:` 显式假设兜底。
- 删除与 checklist 重复的 §2.1 合约/保鲜细则正文；唯一未被 reference 覆盖的
  `default=false` 不算隔离证据反例补进 checklist 版本隔离条目，无语义损失。
- 证据成熟度五级、severity、遗漏复查、honesty notes 保留为单行契约。

## 测试清单

- `cargo test -p opencoder-core --test skill_contract`：23 passed
  （含更新后的 `seeded_task_plan_skill_requires_launch_closure_contract`
  —— 契约短语改钉五要素锚点；`..._question_tool_guidance` 与
  `..._body_unlocks_question_in_prefix_window` 原样通过，`question` 位于
  注入 body 第 377 字符，仍在 500 字符解锁窗口内）。
- `cargo test -p opencoder-core --lib skill`：42 passed（seeding never-clobber、
  frontmatter 解析、reseed 恢复澄清协议）。
