Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# session 模块

会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO；steer 打断进行中 turn、queue 等 idle。

## 索引
- `src/runner/` — drain/执行/sidecar/steer，subagent 在 `runner/subagent.rs`
- `src/tools/` — 工具注册与实现
- `src/bash_guard.rs` — plan/sidecar 只读 bash 门（薄适配 shellguard，fail-closed 见 [shellguard](../shellguard/index.md)）
- `src/compaction/` — 上下文压缩
- `src/resume.rs` — 恢复；取消原语在 `src/lib.rs`
- `tests/` — 集成回归（`bash_guard_plan_mode.rs`、`subagent.rs`、`compaction_*` 等），需登录式环境（`HOME`/`SHELL`）
