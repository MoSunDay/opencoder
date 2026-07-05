Commit: (working-tree, pre-initial-commit)

# 规则 02：迭代回归 Gate

## 核心原则

**每轮迭代结束前必须全量回归 `cargo test --workspace`，并在 changelog 中附「功能 → 测试名」映射。**

回归不通过 = 迭代未完成。

## 迭代收尾检查清单

每轮迭代（iteration）宣告完成前，必须依次执行并通过：

1. **Lint gate**
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```
   零警告。有警告必须修复，不允许 `#[allow]` 绕过（除非有明确技术原因并注释）。

2. **全量测试 gate**
   ```bash
   cargo test --workspace
   ```
   全部通过。不允许用 `#[ignore]` 跳过失败测试来"修绿"。

3. **构建 gate**
   ```bash
   cargo build --workspace
   ```
   零错误。

4. **行数 gate**
   新增文件 ≤ 400 行；迭代中文件 ≤ 800 行。超限必须拆分。

5. **安全 gate**
   无硬编码密钥 / 凭证 / token / 连接串。

6. **文档同步 gate**
   - 触及区的 `agents/*` / `features/*` memory 文档已更新
   - changelog 新增条目，附测试清单（见下）

## Changelog 测试清单格式

每轮迭代的 changelog 必须包含以下小节：

```markdown
## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Store WAL 恢复 | `wal_crash_recovery` | `store/tests/store_integration.rs` |
| 压缩触发 | `compaction_triggers_by_token_estimate` | `session/tests/compaction_and_model.rs` |
| ... | ... | ... |

- 全量回归：`cargo test --workspace` → N passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
```

## 禁止行为

- ❌ 删除测试来让 `cargo test` 通过
- ❌ 用 `#[ignore]` 隐藏失败测试
- ❌ 降低断言严格度来"修绿"（如 `assert!(result.is_ok())` 替代具体值断言）
- ❌ 在 changelog 中虚报测试数量（必须附实际 `cargo test` 输出）
- ❌ 跳过 lint gate（clippy 警告不算"不影响功能"）

## 回归基线

每轮迭代开始时记录当前测试基线：

```
迭代 N 开始基线：cargo test --workspace → {X} passed / 0 failed
迭代 N 结束目标：cargo test --workspace → {X + 新增} passed / 0 failed
```

迭代结束时的测试数必须 ≥ 开始基线 + 本轮新增功能数。测试数下降 = 回归失败。
