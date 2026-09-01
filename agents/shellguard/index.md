Commit: 8709349 (working-tree, plan 严格只读写效应标记)

# shellguard 模块

## 职责
sandbox 模式的 shell 命令安全分类器（`opencoder-shellguard`）。分类核提取自 [rippy](https://github.com/mpecan/rippy)（MIT，版权归 rippy 作者），按 sandbox 策略改造：判定基于 rable 解析出的 AST，而非字符串形状穷举。

## 关键抽象
- `classify(command) -> Verdict`（`src/lib.rs`）：唯一入口。策略——一切携带风险的写拦截；释放集仅 `/dev/null` + `/tmp`；cwd/项目目录**不释放**；不可解析 fail-closed（`Ask`）；`Allow` 放行、`Ask`/`Deny` 一律拦截。
- 管线（`lib.rs` 模块序即数据流向）：`nesting` 界定输入形状 → `parser`（rable）产 AST → `ast` 节点分类 → `resolve` 静态求值 shell 展开 → `analyzer` 逐命令下钻 `handlers/` 注册表；`perl_safety`/`python_safety`/`ruby_safety`/`node_safety`/`sql` 覆盖解释器内嵌代码与 SQL 写面。
- `Verdict`/`Decision`/`AllowReason`（`src/verdict.rs`）：三值判定 + 理由；`Verdict::writes_state` 是组合时不丢失的类型化写效应标记。shellguard 仍可按 sandbox 策略 `Allow` 落在 `/tmp` 的变更，但严格只读消费方可据此继续拦截；精确 `/dev/null` 与 fd redirect 不产生持久状态，标记为 false。

## 质量约束
- crate 级 lint：`unwrap_used`/`expect_used`/`panic` 全 `deny`——分类器自身不允许 panic 路径（`Cargo.toml [lints.clippy]`）。
- 判定兼容性由 session 侧 229 行 / 230 断言表驱动 corpus 守护（`crates/session/src/bash_guard_compat_tests{,2}.rs`，分歧按 RELEASE/RETARGETED/OVER-BLOCK/RELAXED 显式标注）。

## 依赖与接口
- 依赖：`rable`（shell 解析）、thiserror。
- 被依赖：`opencoder-session`（`bash_guard.rs` 严格只读适配：无写效应的 `Allow → ReadOnly`；带 `writes_state` 的 `Allow` 及全部 `Ask`/`Deny → WriteBlocked`）。

## 相关模块
- [agents/session](../session/index.md) — plan 模式严格只读 bash 拦截消费方。
