# 测试补缺：`OPENCODER_CACHE_SALT` 环境覆盖路径

## 背景

`cache_salt`（per-agent 前缀缓存盐，默认 `Some(true)`，控制出站请求体是否带 `cache_salt` 字段）
有两个可设置路径：

1. 文件路径：`opencoder.json` 的 `"cache_salt": bool`（`Config::load` 反序列化）；
2. 环境路径：`OPENCODER_CACHE_SALT` 经 `Config::apply_env`（`crates/core/src/config.rs::apply_env`）
   在反序列化后覆盖，接受 `true/1/yes` 与 `false/0/no`，无法识别的值被忽略。

`cache_salt.rs` 已覆盖出站请求体发射（默认/`=false`/`=true` 三态 + 子 agent 隔离），
但那里的用例直接构造 `Config` 对象设置该字段，**绕过了 `apply_env`**——
即「环境变量 → `Config.cache_salt`」这一接缝此前**零覆盖**。

## 变更

纯测试增量，零生产代码改动（运行时 blast radius = 0）。

- `crates/core/tests/config_contract.rs` 新增集成测试 `cache_salt_env_override`：
  经真实的 `Config::load(dir)` → `apply_env` 路径驱动，断言可观测的 `Config.cache_salt` 状态：
  - 环境未设 → serde 默认 `Some(true)`；
  - `=false` 覆盖文件中的 `true`；
  - truthy 别名（`1`/`yes`/`true`）→ `Some(true)`；
  - falsy 别名（`0`/`no`/`false`）→ `Some(false)`；
  - 无法识别值（`maybe`）→ 忽略，文件 `true` 存活。

  env 变更经既有 `ENV_LOCK` 串行化（与同文件内 `OPENCODER_MODEL`/`ZHIPU_API_KEY` 用例同一约定），
  无时序/网络/DB 依赖。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `OPENCODER_CACHE_SALT` 环境覆盖路径（未设/各别名/覆盖文件/忽略垃圾值） | `cache_salt_env_override` | `crates/core/tests/config_contract.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo build --workspace` | 零错误 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo test --workspace` | 711 passed / 0 failed / 0 ignored |
