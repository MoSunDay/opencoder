Commit: 2aa44247d199d782881b9ee64921c4c6de2e6199

# TUI Codex 会话继承与共享 Harness 构造

TUI 的 `--wrap codex` 与重复 `--envs KEY=VALUE` 使用共享会话运行时。`/task` 新建任务继承启动时的执行器、agent、Codex model 与注入变量；旧 Codex thread ID 不跨任务。恢复时允许保存运行态包含托管配置额外变量，但显式重传的变量值必须匹配。

新会话 Harness 选择集中在 core 的纯函数；Codex 的进程、JSONL 与工具映射仍由 session 的独立适配模块负责。TUI 只持有选择和展示事件。

## 相关

- [TUI 逻辑](../../../agents/tui/index.md)
- [Agent Harness](../../harness/index.md)
