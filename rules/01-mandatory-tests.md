Commit: (working-tree, pre-initial-commit)

# 规则 01：业务功能强制测试

## 核心原则

**每一个业务功能（business feature）都必须有对应的测试用例。** 不是建议，是硬性要求。

没有测试的代码 = 未完成的代码。

## 什么算"业务功能"

以下任一都属于业务功能，必须有测试：

1. **公共函数 / 方法**：`pub fn`、`pub async fn`、trait method（含默认实现）
2. **公共类型的关键行为**：构造、转换、序列化/反序列化、状态变迁
3. **用户可感知的行为**：CLI 子命令、TUI 交互、HTTP 端点、事件流
4. **业务规则**：压缩触发条件、steer/queue 消费顺序、token 估算、配置合并优先级

## 什么不算"有测试"

以下形式**不达标**，视为无测试：

- ❌ 只构造对象、不断言行为（`let x = Foo::new(); assert!(x.is_some())`）
- ❌ 只测 happy path、不测边界（空输入、溢出、并发、错误传播）
- ❌ 复制实现逻辑做自证（`assert_eq!(f(x), f(x))`）
- ❌ 测试依赖网络 / 外部服务且无 Mock 替代
- ❌ `#[ignore]` 的测试（除非有明确原因且记录在案）

## 达标标准

- ✅ 测试断言**可观测的输出**（返回值、状态变迁、持久化结果、事件序列）
- ✅ 至少覆盖一个正常路径 + 一个边界 / 错误路径
- ✅ 纯函数用确定性输入；有副作用的用 Mock（如 `MockChatClient`）或 tempdir
- ✅ 测试名描述行为（`compaction_triggers_when_tokens_exceed_threshold`，不是 `test1`）

## 执行要求

| 场景 | 要求 |
|------|------|
| 新增 `pub fn` | 同 commit 内附带测试 |
| 修改 `pub fn` 行为 | 同 commit 内更新对应测试 |
| 新增 CLI 子命令 | 附带解析测试 + 分发测试 |
| 新增 HTTP 端点 | 附带 HTTP 层测试（非旁路直接调内部函数） |
| 新增 SessionEvent variant | 所有 `match` 处的测试覆盖新 variant |
| 新增 Tool | 附带工具 execute 测试（用 tempdir） |

## 豁免

仅以下情况可豁免测试，但必须在代码中用注释说明原因：

- `Display` / `Debug` 等 derive 宏自动生成的 trait impl
- 纯 I/O 包装（如 `enable_raw_mode`）无法在无终端环境测试
- 上游已充分测试的 re-export（如 `pub use`）

## 违规处理

代码审查时发现无测试的公共函数 → **阻塞合并**，直到补齐测试或获得明确豁免。
