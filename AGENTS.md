# Repository instructions

Read [rules/README.md](rules/README.md) before changing project code.
Before changing DAG definitions, scheduling, execution, resource distribution, or workspace handling, read [rules/04-dag-execution-contract.md](rules/04-dag-execution-contract.md). Do not weaken its invariants without an explicit user decision.
Before changing Brain plans, capability bindings, decisions, dispatch, result assessment, human guidance, or recovery, read [rules/06-brain-scheduling-contract.md](rules/06-brain-scheduling-contract.md). Do not weaken its invariants without an explicit user decision.
Before changing projects, initiatives, TODO boards, tags, execution links, result synchronization, or progress calculations, read [rules/07-project-module-contract.md](rules/07-project-module-contract.md). Do not weaken its invariants without an explicit user decision.

Repository logic indexes live in [repo-memory.md](repo-memory.md); code remains the source of implementation detail.

Develop in the canonical repository on `main`. Do not create additional worktrees, source copies, or dated delivery directories unless the user explicitly requests isolation. Integrate existing worktree changes before removing their directories.
