---
name: review
description: Read-only post-completion assessment that meets the repo go-live standard (rules/01-03). Re-runs the regression gates itself for fresh evidence, checks the test-count baseline didn't drop, scans the diff for green-washing (#[ignore], deleted tests, weakened asserts), verifies test quality + layering + structural coverage, reviews the full diff for scope-creep/secrets/debug output, then rules on go-live readiness. Never edits code or commits.
---

# review —— 上线前评审契约（符合 rules/01-03 go-live 标准）

## 角色
**只读**评估契约。在「自认为做完」之后、提交/上线之前介入。回答四个问题：
1. 是否完成了目标？通过了验证？
2. 验证的方式和证据是什么？（没有证据 = 没有通过）
3. 全局看是否还有问题？改动是否影响到了其它模块？
4. 是否达到了上线（go-live）的标准？

> **只读**：本 skill 不修改代码、不提交、不推送。需要修改时回到 `do-and-done` / 实现循环；需要提交时用 `submit` skill。

## 何时使用
- 自评「做完了」之后，提交之前。
- `do-and-done` 声明 `DONE / go-live ready` 之前作为独立复核。
- 任何时候被要求「看看还有没有问题 / 能不能上线」。

## 输入
- 最近一次 `task-plan` 的 STATUS 块（goal / todos / evidence）——**仅作参考，不盲信**（见下）。
- 工作区实际状态：`git status`、`git diff`、`git diff --cached`。
- 本仓库 go-live gate（rules/02 + rules/01 + rules/03；见下）。

## 验证强化原则（防伪绿 —— 核心）
1. **复跑取证，不信任陈旧证据**：STATUS 块的证据可能来自上一会话或被记错。review 时**自行执行** gate 命令，捕获**当次**输出尾段（测试数、警告数、错误数）作为证据，而非复制 STATUS 块的旧摘要。
2. **回归基线不降**：迭代开始记基线 `cargo test --workspace → X passed`；review 时断言 `当次 passed ≥ 基线 + 本轮新增功能数`。**绿但掉测试 = 静默回归 = ❌**。
3. **绝不伪绿**：没实跑的测试不算通过；没看的回归面不算覆盖；推测的影响不算评估；N/A 必须附一行理由。

## 四维评估

### 1. 目标达成（Goal）
- 逐条比对 STATUS 块的 goal 与 todos，确认每条 `completed` 都有**可追溯证据**。
- 验收口径是否真的满足（不是「我做了」，而是「验收条件命中了」）。

### 2. 验证方式与证据（Validation & Evidence）

**2a. 复跑 gate（自取当次证据）**
```
cargo test --workspace                       # 全绿 + 记录 passed 数
cargo clippy --workspace --all-targets -- -D warnings   # 零警告
cargo build --workspace                      # 零错误
```
捕获每条的输出尾段（`test result: ok. N passed; 0 failed` / `Finished` / 警告数）作证据。

**2b. 回归基线**：当次 `passed` 数 ≥ 迭代开始基线 + 本轮新增功能数；下降即 ❌（rules/02 回归基线）。

**2c. 防修绿 diff 扫描**（rules/02 五禁）：`git diff` 扫以下，任一命中即 ❌ 并要求整改：
- 新增 `#[ignore]`（无注释豁免的）。
- 被删除的 `#[test]` / `#[tokio::test]` 函数（删测试修绿）。
- 新增弱断言：`assert!(.*is_ok())` / `assert!(.*is_some())` / `assert!(true)` 替代了原本的具体值断言。
- 虚报：changelog 测试覆盖表的数字 ≠ 当次实跑输出（见 4）。

**2d. 测试质量**（rules/01 达标标准）：本轮新测试逐条核对——
- 断言**可观测输出**（返回值/状态变迁/持久化/事件序列），不是构造对象就完事。
- 至少一个正常路径 + 一个边界/错误路径。
- 命名描述行为（`shutdown_worker_returns_within_bound...`，不是 `test1`）。
- 不复制实现逻辑自证（`assert_eq!(f(x), f(x))` ❌）。
- 弱测试 = 无测试 → 该功能项退回未验证。

**2e. 测试分层**（rules/03）：新测试放对层——
- unit（源文件内联）：零 I/O / 网络 / DB，<10ms。
- integration（`tests/`）：用 `MockChatClient` + `tempdir` + `open_memory()`，不调真 LLM。
- e2e（`scripts/`）：需 API key，手动/CI，不在常规 `cargo test`。
放错层（如 unit 里开 DB）→ ❌。

**2f. 结构性变更覆盖**（rules/01 执行要求）：本轮若有以下变更，必须附对应测试，否则 ❌：
- 新 / 改行为的 `pub fn` → 同 commit 测试。
- 新 SessionEvent variant → 所有 `match` 处覆盖。
- 新 Tool → execute 测试（tempdir）。
- 新 CLI 子命令 → 解析 + 分发测试。
- 新 HTTP 端点 → HTTP 层测试（非旁路调内部）。

**2g. e2e 适用判定**：变更若触及 **session runner / store 数据形状 / prompt 契约 / 跨进程恢复**等深度契约 → 提示考虑 e2e（`scripts/e2e-glm.sh`）。无 API key 则 soft-skip 并在 REVIEW 注明 `e2e: skipped (no key)`，不强制。

### 3. 全局影响（Global / Blast Radius）

**3a. 依赖图外扩一层**：被改的 trait / 公共函数 / 配置 / 数据形状的调用方是否仍正确？同 crate 单测之外的集成 / e2e / web / cli / 跨 crate 回归面是否覆盖？

**3b. 全量 diff 逐行审**（`git diff` + `git diff --cached`）排查：
- 调试输出：`println!` / `eprintln!` / `dbg!` / `console.log` / `todo!` / `unimplemented!`。
- 注释掉的代码、遗留 `TODO` / `FIXME`。
- 硬编码密钥 / 凭证 / token / 连接串（呼应全局安全约束）。
- **范围外脏改动**：与本任务无关的已改文件（如上一会话遗留）→ 标红列出，询问是否在 scope；**不计入本 review 范围，也不应随本次提交**（提交时由 `submit` skill 排除）。

**3c. 行数 gate**：`wc -l` 检视触改文件——新增 ≤400 / 迭代中 ≤800，超限 → ❌（须拆分）。

**3d. flaky 风险 flag**：新测试含时序断言（`>=Ns && <Ms` 时间窗）/ 并发竞态 / 网络依赖 → 标注 flaky 风险，建议确定性化或附理由（如 `shutdown_worker_is_bounded` 用 ≥3s&&<6s 属可接受防御测试，但需注明）。

### 4. 上线结论（Go-Live Verdict）
对照仓库 gate 逐项给 ✅ / ❌ / N/A（**N/A 必须附一行理由，无理由的 N/A 视为 ❌**）：

- STATUS 块内 TODO 全 completed
- `cargo test --workspace` 全绿（当次实跑，证据=输出尾段）
- **回归基线：当次 passed ≥ 基线 + 新增**（rules/02）
- `cargo clippy --workspace --all-targets -- -D warnings` 零警告（当次实跑）
- `cargo build --workspace` 编译干净（当次实跑）
- 防修绿 diff 扫描无命中（rules/02 五禁）
- 新测试质量达标（rules/01）+ 分层正确（rules/03）+ 结构性变更有覆盖
- 变更代码无遗留 `TODO` / `FIXME` / 调试输出
- 无硬编码密钥 / 凭证
- 文件行数合规（新增 ≤400 / 迭代中 ≤800）
- 触及区 memory 文档已按 `repo-local-memory` repair-on-touch
- changelog 已写，且**测试覆盖表数字 = 当次实跑输出**（rules/02）
- 范围外脏改动已识别并排除（不混入本次范围）

仓库可在 `AGENTS.md` 或 `.opencode/golive.md` 覆盖此清单；存在时优先遵循仓库规则。目录无对应工具的 gate 标 `N/A`（附理由）。

## 固定输出 —— REVIEW 块（每次评审必须输出）
```
## REVIEW
goal_met: yes | partial | no
baseline: <迭代开始 X passed> → 当次 <Y passed>（Y ≥ X + 新增 ? ✅ : ❌）
validation:
  - cargo test --workspace → <PASS|FAIL> ← <当次输出尾段：N passed / 0 failed>
  - cargo clippy ... -D warnings → <PASS|FAIL> ← <零警告 / N 警告>
  - cargo build --workspace → <PASS|FAIL> ← <Finished / error>
  - 防修绿 diff 扫描 → <PASS|FAIL> ← <无命中 / 命中清单>
  - 测试质量(rules/01) → <PASS|FAIL|N/A(理由)> ← <逐条要点>
  - 分层(rules/03) → <PASS|FAIL|N/A(理由)>
  - 结构性变更覆盖 → <PASS|FAIL|N/A(理由)>
evidence_summary: <一句话：证据是否为当次实跑、是否充分>
global:
  - impacted: <模块/文件清单 或 none>
  - regressions_risk: <低|中|高 + 理由>
  - leftover: <TODO/调试输出/密钥 清单 或 none>
  - out_of_scope: <范围外脏改动清单 或 none>
  - flaky_risk: <时序/并发/网络测试 + 理由 或 none>
go_live_gates:
  - <gate 名> → <✅|❌|N/A(理由)> ← <证据或缺口>
changelog_check: <changelog 数字 = 当次实跑 ? ✅ : ❌>
verdict: go-live ready | not ready
gaps: <若 not ready，逐条列出阻塞项与建议；ready 则 none>
```

## 结论规则
- 任一 go-live gate ❌ → `verdict: not ready`，并在 `gaps` 列清阻塞项与建议。
- 全部 ✅ 或附理由 N/A → `verdict: go-live ready`。
- **绝不**伪绿：没实跑的测试不算通过；没看的回归面不算覆盖；推测的影响不算评估；N/A 无理由视为 ❌。
- review 只给结论与缺口，**不自行修改代码或提交**。需要修 → 回实现循环；需要提交 → 用 `submit` skill。

## 与其它 skill 的衔接
- 消费 `task-plan` 的 STATUS 块作参考，但**以自取当次证据为准**。
- 结论 `not ready` 时把 `gaps` 喂回 `do-and-done` 继续推进。
- 结论 `go-live ready` 后，提交动作交给 `submit` skill（提交本身不可逆，按其暂停协议）。
