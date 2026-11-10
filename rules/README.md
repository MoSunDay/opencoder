Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# OpenCoder 仓库规则索引

所有 agent（人工 / AI）在本仓库工作时必须遵循以下规则；文件按编号排列，每条独立可检索。测试组织约定已并入规则 03，Ontology CLI 约定已并入规则 09，原规则 10 / 11 的引用请改指对应文件。

| 编号 | 文件 | 摘要 |
|------|------|------|
| 01 | [mandatory-tests.md](01-mandatory-tests.md) | 每个功能必须有测试；断言可观测输出，禁止只构造对象的表面测试 |
| 02 | [regression-gate.md](02-regression-gate.md) | 只回归本次改动及直接影响链路，禁止默认全量测试；changelog 附测试清单 |
| 03 | [test-pyramid.md](03-test-pyramid.md) | 测试分层与组织：unit/integration/e2e 判层、取证规范、目录落位、夹具复用与验收脚本归组 |
| 04 | [dag-execution-contract.md](04-dag-execution-contract.md) | DAG 单节点单容器、只读 NFS 源工作区、节点本地写层与原生步骤约定 |
| 05 | [ui-acceptance.md](05-ui-acceptance.md) | 按改动范围验证页面功能、屏宽、相关 TUI 与成套构建 |
| 06 | [brain-scheduling-contract.md](06-brain-scheduling-contract.md) | 大脑分层并行、整层评估、返工预算、节点选择、人工引导及恢复约定 |
| 07 | [project-module-contract.md](07-project-module-contract.md) | 项目、专项、TODO 的归属、看板、执行关联、实时结果读取、进度与标签约定 |
| 08 | [infrastructure-scope.md](08-infrastructure-scope.md) | 仓库只保留通用基建能力，代码、样例与文档不得带入具体业务信息 |
| 09 | [ontology-capabilities.md](09-ontology-capabilities.md) | Ontology 能力目录与 CLI：统一入口、同步要求与 opencoder-cli ontology 操作约定 |

## 快速检查清单（每次 PR / commit 前）

- [ ] 修改 Rust 时对相关 crate 执行 Clippy，零警告
- [ ] 本次改动及直接影响链路的定向测试通过；不默认全量回归
- [ ] 新增 / 修改的公共函数有对应测试（规则 01）；changelog 附「功能 → 测试名」映射（规则 02）
- [ ] 无硬编码密钥 / 凭证；代码、样例、提示词与文档只描述通用基建（规则 08）
- [ ] 新增文件 ≤ 400 行；迭代中文件 ≤ 800 行
- [ ] 能力或入口变更已更新 Ontology 登记并做定向验证（规则 09）
- [ ] 新增测试 case 按规则 03 的地图归位，复用既有 support 夹具
