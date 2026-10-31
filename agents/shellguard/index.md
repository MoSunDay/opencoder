Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# shellguard 模块

sandbox shell 命令安全分类器（rable AST 判定）。细节以代码为准。
接缝：session 的 plan/sidecar 只读执行门经此判定 Bash 命令。

## 索引
- [src/lib.rs](../../crates/shellguard/src/lib.rs) — `classify`/`classify_in`
  唯一入口
- [analyzer_dispatch.rs](../../crates/shellguard/src/analyzer_dispatch.rs)
  — 命令分发与 `default_verdict`
- [src/verdict.rs](../../crates/shellguard/src/verdict.rs) — `Verdict`
  （Allow/Ask/Deny）与 `writes_state`
- [src/handlers/](../../crates/shellguard/src/handlers) — 命令 handler 注册表
- [src/allowlists.rs](../../crates/shellguard/src/allowlists.rs)
  — simple-safe 白名单
