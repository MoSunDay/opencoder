Commit: (working-tree, pre-initial-commit)

# 规则 03：测试金字塔分层

## 分层模型

```
        /\
       /e2e\          scripts/e2e-glm.sh
      /------\        真 LLM (glm5.2) 端到端，手动 / CI 触发
     /        \
    /integration\     crates/*/tests/*.rs
   /--------------\   MockChatClient + tempdir + 真 Store
  /                \
 /      unit        \  crates/*/src/*.rs 内 #[cfg(test)]
/--------------------\ 纯函数，零副作用，毫秒级
```

## 各层职责

### 第 1 层：单元测试（unit）

- **位置**：源文件内联 `#[cfg(test)] mod tests`
- **对象**：纯函数、无副作用的逻辑、数据转换
- **要求**：零 I/O、零网络、零全局状态；`< 10ms` / 个
- **示例**：`tokens::estimate`、`composer::cursor_column`、`fmt::format_tokens_compact`、`config::Config::load`

```rust
// crates/tui/src/fmt.rs
#[cfg(test)]
mod tests {
    #[test]
    fn compact_thousands() {
        assert_eq!(format_tokens_compact(12_345), "12.35K");
    }
}
```

### 第 2 层：集成测试（integration）

- **位置**：`crates/<crate>/tests/*.rs`
- **对象**：跨模块协作、持久化、Mock 驱动的业务流程
- **要求**：用 `MockChatClient`（非真网络）、`tempdir`（非真文件系统）、`LibsqlStore::open_memory()`（非真数据库文件）
- **示例**：`steer_followup.rs`、`recovery.rs`、`web_contract.rs`、`store_integration.rs`

```rust
// crates/session/tests/steer_followup.rs
#[tokio::test]
async fn steer_promotes_at_turn_boundary_and_resets_step() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = MockChatClient::new();
    // ... 断言 drain 语义
}
```

### 第 3 层：端到端测试（e2e）

- **位置**：`scripts/e2e-glm.sh` 或 `tests/e2e/`
- **对象**：真 LLM、真文件系统、完整用户流程
- **要求**：需要 API key；标记为手动 / CI 专属；不在常规 `cargo test` 中运行
- **示例**：glm5.2 写贪吃蛇 / 雷霆战机、resume 跨进程

## 放置决策树

```
新功能有副作用吗？
├── 否 → 第 1 层（源文件内联）
└── 是 → 跨多个模块吗？
    ├── 否 → 第 2 层（tests/ 目录，用 Mock）
    └── 是 → 需要真外部服务吗？
        ├── 否 → 第 2 层（tests/ 目录，用 Mock + tempdir）
        └── 是 → 第 3 层（scripts/ 或 tests/e2e/）
```

## 禁止行为

- ❌ 在单元测试中打开文件 / 网络 / 数据库
- ❌ 在集成测试中调用真 LLM API（用 `MockChatClient`）
- ❌ 把所有测试堆在一个巨型 `tests/main.rs` 里（按功能拆分文件）
- ❌ 测试文件超过 400 行（拆分或提取 helper）

## Mock 使用规范

- `MockChatClient`：FIFO 脚本回放 + 请求体录制（`crates/llm/src/mock.rs`）
- `LibsqlStore::open_memory()`：内存 SQLite，零文件残留
- `tempfile::TempDir`：文件系统操作的隔离临时目录
- 不要 Mock 自己的内部类型（只 Mock 外部边界：LLM、文件系统、网络）
