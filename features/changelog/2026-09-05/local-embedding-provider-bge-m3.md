# 本机 llama → 本地 embedding 模型对接：`embedding_provider` 独立路由 + ollama bge-m3

## Context

brain 能力库的语义检索/动态规划调度依赖 `/embeddings`（`ChatStream::embed`），此前嵌入**只能**走主 provider 端点（`resolve_endpoint`）。本机原 `local` provider 是 llama 系 27B 对话模型（`192.168.31.159:8180`，qwen3.8-27b），既不适合做嵌入也不在运行。本轮把它替换为**本地 embedding 模型服务**（ollama + bge-m3，1024 维，中文/跨语言友好），并给 Config 补上「嵌入走独立 provider」的路由能力，使对话保持远程 glm、嵌入走本地模型服务互不干扰。

## Change Summary

- **core `Config`**：新增 `embedding_provider: Option<String>` 字段（serde/merge 双路径齐备：`merge.rs` 的 `has_editable_key` 与 `merge_into` 各加一处分支）+ `resolve_embedding_endpoint()`——`None`/等于主 provider → 回落 `resolve_endpoint()`；`Some(name)` → 该注册 provider 的 base_url/api_key/headers。未注册名是**点名报错**而非静默回落（异模型向量不可比，静默打错服务器属数据完整性 bug）。`embedding_model_id` 文档同步。
- **web brain 生产装配**（`crates/web/src/lib.rs`）：brain Runtime 的嵌入客户端从 `resolve_endpoint()` 改为 `resolve_embedding_endpoint()`；解析失败仍降级 `degraded_brain`，serve 照常启动。
- **机器侧对接（非仓库内产物）**：
  - `ollama-embed.service`（systemd 常驻，`OLLAMA_HOST=127.0.0.1:11434`，`OLLAMA_FLASH_ATTENTION=0`——**绕开 ollama 0.22.1 bge-m3 flash-attention fp16 NaN bug**：特定中英混排文本会产出含 NaN 的向量，Go 端无法 JSON 编码直接 500）。
  - `~/.opencoder/config.json`：`providers.local` 改指 `http://127.0.0.1:11434/v1`（model `bge-m3`），顶层 `embedding_model: "bge-m3"` + `embedding_provider: "local"`；对话仍走 `glm-5.2/glm-5.3`。

## 测试与回归证据

- `crates/core/tests/config_providers.rs`（+3 例，20 全绿）：专用 provider 路由（chat 端点不受影响 + headers/api_key/model_id 各自就位）；无 `embedding_provider` 时回落主 provider；未注册名报错点名且不影响 chat 解析。
- 真机 e2e（`opencode-server` + 真 ollama bge-m3 + HMAC 签名 HTTP）：创建两条中文能力 201（真 1024 维向量落库）→ 中文查询「怎么排查段错误崩溃」Top1 命中 gdb 能力、英文查询 "write a retention SQL query" Top1 命中 SQL 能力（跨语言语义检索）→ 清理 200，库留空。bge-m3 区分度实测：同义中英 0.844 vs 无关 0.406。
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0。
- `cargo test --workspace --no-fail-fast` → 见下方回填。

- `cargo test --workspace --no-fail-fast` → exit=0：302 suite ok / 0 failed / 4416 passed。

## 设计取舍备忘

- **独立 `embedding_provider` 而非复用 primary**：对话与嵌入的算力/端点天然分离（远程 API vs 本地 GPU），主 provider 换模型不应隐式更换嵌入模型（向量维度/语义空间变化会让既有库全部失配）。
- **未知 provider 报错不回落**：向量混源是静默数据损坏，宁可 502 暴露。
- **flash attention 关闭**：嵌入是短文本批处理，FA 收益近零，NaN 风险为零更重要；若后续 ollama 修复该 bug 可移除该环境变量。
