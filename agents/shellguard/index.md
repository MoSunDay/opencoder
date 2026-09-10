Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# shellguard 模块

sandbox shell 命令安全分类器（rable AST 判定）。

## 关键路径
- `src/lib.rs::classify / classify_in` — 唯一入口；`classify_in` 显式 cwd，须对齐执行 cwd。
- `src/lib.rs` — 不可解析 fail-closed：一律 `Ask` 拦截。
- `src/verdict.rs` — `Decision::{Allow,Ask,Deny}` 三值 + `writes_state` 写效应标记。
- `src/handlers/scope.rs` — 释放集仅 `/dev/null` + `/tmp`；cwd/项目目录不释放。
- `src/lib.rs` 管线 — nesting → parser（rable）→ ast → resolve → analyzer。
- `src/handlers/` — 每命令 handler 注册表（git/docker/curl/sed/cloud/…）。
- `src/{perl,python,ruby,node}_safety.rs`、`src/sql.rs` — 解释器内嵌代码与 SQL 写面。
- `src/allowlists.rs` — simple-safe 白名单（rippy 数据裁剪）。
- `Cargo.toml [lints.clippy]` — `unwrap_used`/`expect_used`/`panic` 全 deny。
- `crates/session/src/bash_guard_compat_tests{,2}.rs` — 表驱动判定 corpus。

## 边界
- `Allow` 落 /tmp 的变更仍带 `writes_state`，严格只读消费方据此继续拦。
- `cd` 静态可解析纯导航 Allow 并重瞄判定 cwd；不可解析目标及 pushd/popd 均 Ask。

## 相关
- [agents/session](../session/index.md) — bash_guard 严格只读适配消费方。
