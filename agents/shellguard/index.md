Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# shellguard 模块

sandbox shell 命令安全分类器（rable AST 判定）；释放集仅 `/tmp` + `/dev/null`，判定 cwd 对齐执行 cwd。

## 索引
- `src/lib.rs::classify/classify_in` — 唯一入口，不可解析 fail-closed 一律 Ask
- `src/analyzer_dispatch.rs::default_verdict` — unknown 命令 allow-by-default（`AllowReason::UnknownCommand`）
- `src/verdict.rs` — Allow/Ask/Deny + `writes_state`
- `src/handlers/` — 每命令 handler 注册表
- `src/allowlists.rs` — simple-safe 白名单
