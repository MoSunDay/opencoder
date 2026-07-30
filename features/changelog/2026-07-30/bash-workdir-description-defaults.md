# bash 工具：workdir 参数描述补充默认值说明

## 背景

bash 工具 `workdir` 参数面向模型的描述原为 `Optional working directory override.`，未说明该参数默认等于会话工作目录。这使模型倾向于在每个命令前手动拼接 `cd <dir> &&`，产生冗余命令，且可能与 workdir 机制重复。

## 改动

将 `BashTool::parameters()` 中 `workdir` 属性的描述改为：

> Optional working directory override. Defaults to the session working directory, so only pass this to run a command in a different directory; no need for a manual `cd`.

明确两点：
1. `workdir` 默认为会话工作目录；
2. 仅在需要切换到其他目录时才传入，无需手动 `cd`。

## 影响

- 仅修改 `parameters()` 返回的 schema 描述字符串，**不进入任何执行路径**：`execute()` 不读取该描述，运行时行为零变化。
- 文本为 ASCII（符合「默认 ASCII」规则），含反引号代码标记。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| workdir 属性仍暴露于 schema（与 command/timeout 约束一并守护） | `parameters_schema_hides_timeout_from_model` | `crates/session/src/tools/bash.rs`（unit） |

> 本改动为纯描述文本调整，无可执行行为，故无新增行为测试。schema 守卫断言 `workdir` 属性存在性（`.is_some()`），不校验描述文本，故描述变化不影响该测试。

## 验证

- 全仓库无任何测试断言旧描述字符串（仅 `prop_str` 调用本身引用该串），故改动不可能破坏编译或任何测试。
- `cargo test -p opencoder-session --lib parameters_schema_hides_timeout_from_model` -> **1 passed / 0 failed**（opencoder-session lib 编译通过）。
