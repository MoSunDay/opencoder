Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# OpenCoder 仓库规则索引

所有 agent（人工 / AI）在本仓库工作时必须遵循以下规则。规则文件按编号排列，每条独立可检索。

| 编号 | 文件 | 摘要 |
|------|------|------|
| 01 | [mandatory-tests.md](01-mandatory-tests.md) | 每个功能必须有对应测试用例；禁止"只构造对象"的表面测试 |
| 02 | [regression-gate.md](02-regression-gate.md) | 每轮迭代结束前必须全量回归 `cargo test --workspace` |
| 03 | [test-pyramid.md](03-test-pyramid.md) | 测试分层规范：纯函数内联 / 集成放 tests/ / e2e 放 scripts/ |
| 04 | [dag-execution-contract.md](04-dag-execution-contract.md) | DAG 单节点单容器、只读 NFS 源工作区、节点本地写层与原生步骤约定 |
| 05 | [ui-acceptance.md](05-ui-acceptance.md) | 全站功能、四种屏宽、Server TUI 与成套构建的 UI 验收约定 |
| 06 | [brain-scheduling-contract.md](06-brain-scheduling-contract.md) | 大脑分层并行、整层评估、返工预算、节点选择、人工引导及恢复约定 |
| 07 | [project-module-contract.md](07-project-module-contract.md) | 项目、专项、TODO 的归属、看板、执行关联、实时结果读取、进度与标签约定 |

## 快速检查清单（每次 PR / commit 前）

- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 零警告
- [ ] `cargo test --workspace` 全通过
- [ ] 新增 / 修改的公共函数有对应测试（规则 01）
- [ ] 新增功能在 changelog 中附「功能 → 测试名」映射（规则 02）
- [ ] 无硬编码密钥 / 凭证
- [ ] 新增文件 ≤ 400 行；迭代中文件 ≤ 800 行
