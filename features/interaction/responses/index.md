Commit: 573bb2e48fa20832a47b7bec8adc8ee3c08e8e78

# Responses API

Responses 协议的编码工作流：provider state 随消息持久化，resume/fork/
compaction 保持完整工具组与续聊 output。

## 索引

- [llm 域卡](../../../agents/foundation/llm/index.md) — 协议路由
- [crates/llm/src/responses/](../../../crates/llm/src/responses) — request/
  decode/output 编解码；`LlmEvent::ProviderState` 在
  [decode.rs](../../../crates/llm/src/responses/decode.rs)
- [crates/core/src/message.rs](../../../crates/core/src/message.rs) — provider
  state 消息块
- [crates/session/src/runner/mod.rs](../../../crates/session/src/runner/mod.rs)、
  [llm_call.rs](../../../crates/session/src/runner/llm_call.rs) — provider state
  消费与随消息持久化

## 相关

- [agents/llm](../../../agents/foundation/llm/index.md) — 协议路由
- [agents/session](../../../agents/runtime/session/index.md) — 状态持久化
