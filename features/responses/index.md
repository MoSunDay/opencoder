Commit: (working-tree, 基于 0a502305e42a390ba5949ca66211d2d2816512ba)

# Responses API

OpenCoder 可通过 OpenAI Responses API 使用 GPT-5、GPT-6 系列模型，也支持实现同一协议的网关。模型 ID 直接使用服务端实际提供的名称；客户端没有模型白名单。

在项目的 `.opencoder/config.json` 或已有全局配置中设置：

```json
{
  "model": "openai/gpt-5",
  "reasoning_effort": "medium",
  "max_tokens": 16384,
  "providers": {
    "openai": {
      "protocol": "responses",
      "base_url": "https://api.openai.com/v1",
      "api_key": "{OPENAI_API_KEY}"
    }
  }
}
```

将 `gpt-5` 替换为账号或网关提供的 GPT-6 模型 ID 即可切换。`base_url` 是 API 根路径，客户端追加 `/responses`；网关地址、环境变量凭证及自定义 headers 沿用原配置方式。

每个 provider 独立选择 `responses` 或 `chat_completions`。未填写 `protocol` 的旧配置继续使用 Chat Completions；错误的协议值在加载、保存或请求路由时直接报错。TUI `/model` 的 Provider 表单可切换协议。`small_model` 可指向另一个 provider，标题、VERIFY 和压缩请求按各自模型配置路由。

## 编码工作流

- 文本流、推理摘要、拒答、多个并行函数调用与分片参数解析。
- 内建工具和已注册 MCP 工具；工具结果按 `call_id` 回传，保留错误标志及图片。
- 用户图片、工具图片、多轮上下文、子代理，以及 CLI、TUI、Web 和节点执行入口。
- 恢复、分叉、导出/导入、上下文压缩；压缩边界保持工具调用及其结果完整。
- 输入、输出、缓存命中、缓存写入及 reasoning token 用量记录。

Responses 请求使用 `store:false`，本地保存每轮有序的原始 output，包括 reasoning 的 `encrypted_content`、消息 `phase` 和工具 ID。同一 provider、端点与模型的后续轮次原样回传；切换到其他模型或服务时使用通用消息重建上下文。关闭 `interleaved_thinking` 不会丢失 Responses 的续聊状态或推理摘要。旧数据库自动升级，既有消息不变。

`reasoning_effort` 映射到 `reasoning.effort`；留空表示服务端默认，`none` 是明确请求关闭推理，两者不同。TUI 支持 `minimal/low/medium/high/xhigh/max`，保留其他已有配置值；具体允许值由模型服务校验。`max_tokens` 映射到 `max_output_tokens`，包含推理消耗，因此需要为最终答案预留空间。Responses 请求省略 `temperature`。标题和 VERIFY 固定使用 `low` 与 4096 输出预算。

仅成功的 `response.completed` 或完整 JSON Response 才能提交助手消息、执行工具。失败、取消、不完整响应、非法参数和不支持的输出类型直接报错；传输中断在重试预算内重新生成，重试时清除当前尝试的文本和推理，保留此前完成的工具结果。耗尽重试后不会执行部分工具调用。

本功能覆盖 OpenCoder 的编码工作流。OpenAI 托管工具、后台任务、WebSocket 及服务端会话管理不在本次范围。

## 验收

自动验收使用本地 HTTP/SSE 模拟服务、真实 session/store、内建读写与 shell 工具，不访问外部 LLM。协议用例在 `crates/llm/tests/responses/`，会话用例在 `crates/session/tests/responses/`。真实 GPT-5/6 和具体网关的在线验收尚未执行。

实现边界见 [模型客户端](../../agents/llm/index.md)、[会话](../../agents/session/index.md) 和 [持久化](../../agents/store/index.md)。
