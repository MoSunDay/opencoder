Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# OpenCoder 仓库规则索引

所有 agent（人工 / AI）在本仓库工作时必须遵循以下规则。规则文件按编号排列，每条独立可检索。

| 编号 | 文件 | 摘要 |
|------|------|------|
| 01 | [mandatory-tests.md](01-mandatory-tests.md) | 每个功能必须有对应测试用例；禁止"只构造对象"的表面测试 |
| 02 | [regression-gate.md](02-regression-gate.md) | 只回归本次改动及直接影响的链路，禁止默认全量测试 |
| 03 | [test-pyramid.md](03-test-pyramid.md) | 测试分层规范：纯函数内联 / 集成放 tests/ / e2e 放 scripts/ |
| 04 | [dag-execution-contract.md](04-dag-execution-contract.md) | DAG 单节点单容器、只读 NFS 源工作区、节点本地写层与原生步骤约定 |
| 05 | [ui-acceptance.md](05-ui-acceptance.md) | 按改动范围验证页面功能、屏宽、相关 TUI 与成套构建 |
| 06 | [brain-scheduling-contract.md](06-brain-scheduling-contract.md) | 大脑分层并行、整层评估、返工预算、节点选择、人工引导及恢复约定 |
| 07 | [project-module-contract.md](07-project-module-contract.md) | 项目、专项、TODO 的归属、看板、执行关联、实时结果读取、进度与标签约定 |
| 08 | [infrastructure-scope.md](08-infrastructure-scope.md) | 仓库只保留通用基建能力，代码、样例与文档不得带入具体业务信息 |
| 09 | [ontology-capabilities.md](09-ontology-capabilities.md) | Ontology 统一入口、实体目录与关系组织、能力变更同步要求 |
| 10 | [ontology-cli.md](10-ontology-cli.md) | Ontology CLI 增删改查、切面、实体和关系查询、实体目录挂载及同步操作 |

## 快速检查清单（每次 PR / commit 前）

- [ ] 修改 Rust 时对相关 crate 执行 Clippy，零警告
- [ ] 本次改动及直接影响链路的定向测试通过；不默认全量回归
- [ ] 新增 / 修改的公共函数有对应测试（规则 01）
- [ ] 新增功能在 changelog 中附「功能 → 测试名」映射（规则 02）
- [ ] 无硬编码密钥 / 凭证
- [ ] 代码、注释、样例、提示词与文档只描述通用基建，没有具体业务信息或专用逻辑（规则 08）
- [ ] 新增文件 ≤ 400 行；迭代中文件 ≤ 800 行
- [ ] 能力或入口变更已更新 Ontology 登记、同步映射与定向验证
