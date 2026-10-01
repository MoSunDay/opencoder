Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 嵌入模型使用独立 Provider

能力库的语义检索依赖 `ChatStream::embed`。`embedding_provider` 允许对话和嵌入分别使用已注册的 Provider，避免切换对话模型时隐式改变已有向量的来源。

## 变更

- `Config` 新增 `embedding_provider: Option<String>`，序列化与配置合并保持一致。
- `resolve_embedding_endpoint()` 在未指定独立 Provider 或指定主 Provider 时复用对话端点；指定其他已注册 Provider 时读取其地址、认证和请求头。
- 未注册的 Provider 明确报错，不静默回落到其他模型；对话端点的解析不受影响。
- Brain 装配使用独立嵌入端点；解析失败时保持显式降级，不阻止其他服务启动。

## 测试覆盖

| 功能 | 测试 | 文件 |
|------|------|------|
| 独立嵌入路由 | `embedding_provider_routes_embeddings_to_dedicated_endpoint` | `crates/core/tests/config_providers.rs` |
| 未指定独立 Provider 时复用主端点 | `embedding_endpoint_defaults_to_primary_without_embedding_provider` | `crates/core/tests/config_providers.rs` |
| 未注册的 Provider 明确报错 | `unknown_embedding_provider_is_an_error_naming_it` | `crates/core/tests/config_providers.rs` |

- 当时配置测试：20 项通过；workspace 全量回归：302 个套件、4416 passed / 0 failed。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。

本机模型服务的安装配置、部署地址和现场检索记录已外移；这里只记录 OpenCoder 的 Provider 路由契约。
