Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# shellguard 模块

sandbox shell 命令安全分类器（rable AST 判定）。

## 索引
- `src/lib.rs::classify/classify_in` — 唯一入口；不可解析 fail-closed 一律 Ask
- `src/analyzer_dispatch.rs::default_verdict` — 未注册命令（unknown）allow-by-default，出处类型化为 `AllowReason::UnknownCommand`
- `src/verdict.rs` — Allow/Ask/Deny + writes_state
- `src/handlers/` — 每命令 handler 注册表
- `src/allowlists.rs` — simple-safe 白名单

## 边界
- 释放集仅 `/tmp` + `/dev/null`；判定 cwd 必须对齐执行 cwd。
- unknown 命令默认放行（2026-09-20 策略，plan/sidecar 消费方同步放宽）：仅已注册写命令、写重定向、解释器负载、`$()` 替换与不可解析输入 fail-closed。
