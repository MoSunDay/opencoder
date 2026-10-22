# Repository instructions

Read [rules/README.md](rules/README.md) before changing project code.
OpenCoder 只提供通用基建能力。代码、注释、测试、样例、提示词、文档、仓库记忆和变更记录不得带入具体业务的名称、流程、数据或接入细节；所有改动遵循 [rules/08-infrastructure-scope.md](rules/08-infrastructure-scope.md)。
Before changing DAG definitions, scheduling, execution, resource distribution, or workspace handling, read [rules/04-dag-execution-contract.md](rules/04-dag-execution-contract.md). Do not weaken its invariants without an explicit user decision.
Before changing Brain plans, capability bindings, decisions, dispatch, result assessment, human guidance, or recovery, read [rules/06-brain-scheduling-contract.md](rules/06-brain-scheduling-contract.md). Do not weaken its invariants without an explicit user decision.
Before changing projects, initiatives, TODO boards, tags, execution links, result synchronization, or progress calculations, read [rules/07-project-module-contract.md](rules/07-project-module-contract.md). Do not weaken its invariants without an explicit user decision.

Repository logic indexes live in [repo-memory.md](repo-memory.md); code remains the source of implementation detail.

Before changing public capabilities, CLI/API entrypoints, input/output requirements, or dependencies, read [rules/09-ontology-capabilities.md](rules/09-ontology-capabilities.md). Keep the Ontology catalog and affected synchronization tests consistent with the native implementation.
查找、理解和组织现有能力时，从 Ontology 开始；使用或修改 Ontology CLI 前阅读 [rules/10-ontology-cli.md](rules/10-ontology-cli.md)，遵循实体目录挂载、版本校验、软删除和同步检查约定。

## 测试范围

- 改到哪里测哪里：只运行本次改动及其直接影响链路的定向测试，发布时也遵循此规则。
- 不默认运行全量测试，包括 `cargo test --workspace`、SPA 全量测试和全站 UI 验收；只有用户明确要求全量测试时才执行。
- 根据改动选择相关模块、测试文件或用例；只有具体失败或明确的依赖影响需要时，才扩大到相关范围。已通过且代码未再变化的检查不重复运行。
- 记录实际测试范围和结果，不把局部通过写成全量通过。文档改动只检查内容和链接；构建、发布产物核验及线上健康检查按实际需要执行。
