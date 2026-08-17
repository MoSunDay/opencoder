---
name: task-plan
description: Global planning at the start of a task. Decomposes a deliverable goal into concrete TODOs, each with an acceptance plan and dependency-impact check, then emits a STATUS block (goal / todos / progress / gate) that do-and-done consumes, review cross-checks, and submit/summary cite. Read-only — no code edits or commits. The default go-live gate checklist lives here (overridable via AGENTS.md / .opencode/golive.md).
---

# task-plan —— 全局规划契约（产出 STATUS 块，对接 do-and-done / review）

## 角色
规划契约。在动手实现之前介入，把一个交付目标拆解为**可验证、可执行**的 TODO 清单，并为每条 TODO 绑定验收方案与依赖影响分析。产出的 **STATUS 块** 是整条工作流的唯一事实来源：`do-and-done` 消费它推进、`review` 交叉核对它、`submit` / `summary` 引用它。

> **只读**：本 skill 不修改代码、不提交、不推送。它只做规划与 STATUS 块的维护。需要实现时切到 `do-and-done`。

## 何时使用
- 接到新任务 / 需求的第一时间 —— 在 `do-and-done` 之前。
- 任务中途**范围或目标发生变化**时，重新加载并重新规划（再入）。
- 发现当前 TODO 清单不完整 / 验收标准缺失 / 影响面未评估时，回退到此补全。
- 被要求「先出个方案 / 规划 / 排期」时。

## 输入
- 原始需求（用户 prompt / issue / 任务描述）—— 提取**交付目标 + 隐含约束**。
- 仓库实际状态：目录结构、依赖关系（`Cargo.toml` / `package.json`）、既有测试与规则（`rules/`、`AGENTS.md`、`.opencode/golive.md`）。
- 受影响模块的**公开接口**（`pub fn` / trait / HTTP 路由 / CLI 子命令 / SessionEvent 变体）—— 用于影响分析。

## 规划四步法
1. **目标澄清** —— 一句话写清交付物 + 可观测的验收标准（不是「优化体验」，而是「X 场景下 Y 指标达到 Z」）。真歧义按下方「澄清协议」处理，不自行脑补。
2. **拆解 TODO** —— 按**功能边界**拆（高内聚低耦合），每条 TODO 满足：
   - 可独立验证（有自己的验收方案）。
   - 粒度适中：一条 TODO 对应一个可交付的功能点，不混多个职责。
   - 标注依赖关系（若 TODO-B 依赖 TODO-A，显式写出 `depends: A`）。
3. **验收方案绑定** —— 每条 TODO 必须有 `accept:` 字段，写明：
   - 验证命令（如 `cargo test <name>`）+ 期望结果。
   - 或可观测行为（UI 交互 / HTTP 响应 / 事件序列）。
   - **无验收方案的 TODO 不允许进入清单**（无法判断是否做完 = 无法交付）。
4. **依赖影响分析** —— 每条 TODO 必须有 `impact:` 字段，写明：
   - 受影响的模块 / 公开接口 / 下游消费者。
   - 是否**只新增**（零影响，标 `impact: none`）还是**改动既有**（需回归验证）。
   - 若改动既有接口 / 公共抽象 → 标注**必须回归的模块**，拆出专门的回归验证 TODO。
   - **原则：不得破坏既有功能点。** 改动既有契约时，必须同步规划兼容性处理或迁移。

## 澄清协议（目标含糊 / 需求冲突时）
仅当答案是**会改变拆解方向**的真歧义才触发（验收标准二选一、目标互相矛盾、关键约束缺失）；能从仓库 / `rules/` / 既有测试查到的事实一律先查再定，不把提问当侦察手段。
- `question` 工具可用（plan agent 交互式 TUI）→ 调用 `question` 向用户澄清：每次一句一个最关键问题，可附 ≤4 个候选选项，可在同一轮多问；拿到答复再产出 STATUS 块。
- 不可用（非交互 `run` / headless，工具会即刻返回「无监听」应答而不阻塞）→ **显式假设继续规划**：把推断逐条写进 STATUS 块的 `assumptions:`，选最小意外解释，并在 gate 汇报标注「规划基于假设」；绝不静默编造验收标准或替用户拍板不可逆取舍。

## STATUS 块（固定输出格式，每次规划/更新必须产出）
```
## STATUS
goal: <一句话交付目标 + 验收标准>
progress: <0-100>%   ← completed 的 TODO 数 / 总数（向下取整）；无证据项不算 completed
baseline: <迭代开始 cargo test --workspace → X passed>   ← 首次规划时记录，后续不变
todos:
  - [pending] <TODO 描述> | accept: <验收方案> | impact: <受影响模块 / none> | depends: <前置TODO / ->
  - [in_progress] <TODO 描述> | accept: <验收方案> | impact: <...> | reason: <为何未完成>
  - [completed] <TODO 描述> | accept: <验收方案> | evidence: <命令+结果 / file:line / 日志摘要>
gate:                        ← 默认 go-live 清单（下方「默认 gate」）；仓库有覆盖时优先仓库
  - 测试覆盖(rules/01) → <pending|green|na(理由)>
  - 回归不降(rules/02) → <pending|green|na(理由)>
  - 测试分层(rules/03) → <pending|green|na(理由)>
  - clippy 零警告 → <pending|green>
  - 构建干净 → <pending|green>
  - 行数限制(新增≤400/迭代≤800) → <pending|green>
  - 无密钥泄露 → <pending|green>
  - 文档同步(agents/*/features/*) → <pending|green|na(理由)>
assumptions:                  ← 仅澄清协议走「显式假设」时产出；逐条列出假设及其最小意外依据（可省略）
```

### TODO 状态机
- `pending` —— 未开始。新 TODO 默认此态。
- `in_progress` —— 正在做，但**证据不足**（尚未验证 / 验证未通过）。**不计入 progress%**。
- `completed` —— 已验证，附可追溯 `evidence`。计入 progress%。
- 状态流转由 `do-and-done` 在实现循环中更新；task-plan 在重新规划时刷新整体 `progress%` 与 `gate` 状态。

### progress% 计算规则
`progress% = floor(completed 数 / 总 TODO 数 × 100)`。
- 只有 `completed`（有 evidence）计入分子。
- `in_progress` / `pending` 不计入 —— **无证据不算进度，杜绝伪绿。**
- 100% 的唯一判定者是 task-plan（do-and-done 刷新 STATUS 块时调 task-plan 重算）。

## 默认 go-live gate 清单（本 skill 拥有，可被仓库覆盖）
以下为默认 gate，对应 `rules/01-03` + `review` 的 `go_live_gates`。仓库可在 `AGENTS.md` 或 `.opencode/golive.md` **覆盖**此清单（存在时优先遵循仓库规则）：

| gate | 来源 | 要求 |
|---|---|---|
| 测试覆盖 | rules/01 | 每个 pub fn / 业务功能点有测试；断言可观测输出；≥1 正常 + ≥1 边界/错误路径 |
| 结构性变更覆盖 | rules/01 | 新 pub fn→同提交测试；新 CLI→parse+dispatch 测试；新 HTTP→HTTP 层测试；新 SessionEvent→match 覆盖；新 Tool→execute 测试 |
| 回归不降 | rules/02 | `当次 passed ≥ baseline + 本轮新增功能数`；测试数下降 = 回归失败 |
| 测试分层 | rules/03 | unit(零 I/O, <10ms) / integration(MockChatClient+tempdir) / e2e(手动/CI)；不串层 |
| clippy 零警告 | rules/02 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 构建干净 | rules/02 | `cargo build --workspace` |
| 行数限制 | 工程要求 | 新增文件 ≤400 行；迭代中文件 ≤800 行；超限必须拆分 |
| 无密钥泄露 | 安全约束 | 代码中无 API Key / Secret / Token / 密码 / 连接串 / 私钥 |
| 文档同步 | rules/02 | 触及区 `agents/*` / `features/*` 按 repo-local-memory 更新；附 changelog |

**五禁（rules/02）**：❌ 删测试过 gate；❌ `#[ignore]` 藏失败；❌ 弱化断言修绿；❌ changelog 造假数；❌ 跳 lint。

## 依赖影响分析（强制 —— 不得省略）
本次任务的**任一 TODO 若改动既有公开接口 / 公共抽象 / 共享状态**，必须在 STATUS 块外补充一段**影响分析**：

### 影响分析块
```
## 影响分析
changed_contract: <改动的既有接口/抽象/状态，或 none>
affected_modules:
  - <模块> → <受影响的功能点 + 必须回归的验证>
regression_plan:
  - <为受影响模块补的回归验证 TODO（已并入 todos 清单）>
safety: <为何本改动不会破坏既有功能 / 若无法保证，标注风险并拆 TODO>
```
- **只新增、不改既有** → `changed_contract: none`，影响分析可简写，但仍须产出（明示零影响）。
- **改动既有** → 必须逐模块列受影响功能点 + 回归验证 TODO。**无法保证不破坏既有功能时，不得标 safety: safe，须拆出验证 TODO。**

## 与其它 skill 的衔接
- **下游 `do-and-done`**：消费 STATUS 块的 `todos`，逐条实现 → 更新状态 → 调 task-plan 重算 progress%。task-plan 是 TODO 清单的**唯一作者**，do-and-done 不另起清单。
- **下游 `review`**：交叉核对 STATUS 块的 `goal` 与 `todos`，但**自行复跑 gate 取当次证据**（不盲信 STATUS 的 evidence）。task-plan 的 gate 清单 = review 的 `go_live_gates` 来源。
- **下游 `submit`**：引用 STATUS 块的变更汇总作为 changelog / commit message 素材。
- **正交 `summary`**：STATUS 块的 `goal / todos / evidence` 是 summary 的需求来源。
- **重新规划**：范围 / 目标变化时，立刻回到 task-plan 重新拆解（再入），不就地修补 do-and-done 的清单。

## 原则
- **可验证优先**：没有验收方案的 TODO = 没有规划。宁可少拆，不可拆出无法验证的项。
- **影响可见**：每条 TODO 都要回答「这会不会影响别人」。改动既有契约必须显式标注并规划回归。
- **不伪绿**：progress% 只认 completed（有 evidence）；没验证的不算进度。
- **gate 可覆盖**：默认清单服从仓库的 `AGENTS.md` / `.opencode/golive.md`（存在时优先）。
- **证据是建议性的**：task-plan 记录的 evidence 供 do-and-done 推进与 submit 引用；**review 是 go-live 的事实权威**，会自行复跑 gate。
