Commit: (working-tree)

# fix(llm): 同帧 reasoning/content 按语义顺序发射

## Context

部分 OpenAI 兼容服务会在同一个流式 delta 中返回最后一个
`reasoning_content` token 和第一个 `content` token。客户端此前先发正文再发思考，产生
`text("Now") → reasoning(".") → text(" continue")` 一类错序：句末标点被显示成新的
Thinking 块，错误频道还可能随工具轮次持久化并回传模型。

## Change Summary

- 扁平 delta 同时存在 reasoning 与字符串 `content` 时，先发 `ReasoningDelta`，再发
  `TextDelta`。
- 不修改任何 token，不使用标点或文本内容启发式；结构化 `content` 数组继续严格遵循
  数组中的显式顺序。
- 增加双字段 delta 回归测试，锁定事件顺序和正文缓冲内容。

## Impact Surface

- 修复 Thinking 的异常断块和句末标点错位。
- 修复工具轮次中 reasoning/text 错分后持久化、回传模型的风险。
- Viking 代理、公开 API、配置、存储结构及合法的交错思考行为不变。

## Related Docs

- [LLM 模块](../../../agents/llm/index.md)
