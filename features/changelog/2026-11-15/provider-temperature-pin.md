# 辅助调用去温度硬编 + Provider 可选温度钉住：修复仅接受单一 temperature 的模型后端 400

## 背景

部分模型后端（如锁定推理模型）只接受固定采样温度（如仅允许 `1`），
其余取值一律被服务端以 `invalid temperature` 400 拒绝。内部辅助调用
（压缩 0.2 / 标题 0.3 / 验证 0.0）携带硬编码低温，命中此类后端时
`compaction failed` / `stream failed`，且逐轮重试全部失败。

## 改动

- **去掉三处温度硬编**（主修复）：压缩/标题/验证请求一律不发送
  `temperature`，由服务端按模型套用默认值——默认值必然是该模型合法取值，
  是唯一普适安全的选择；与主对话请求及 `responses` 协议路径行为统一。
  质量代价可忽略：摘要/标题对温度不敏感，验证本就有不可解析重试预算。
- `ProviderConfig` 新增可选 `temperature` 字段（纯调优，非必需）：
  为该 provider 全部调用钉住采样温度（如追求可复现时钉 `0`）。解析顺序
  与 `api_key_for` 对齐：`providers[name].temperature` → 旧版顶层
  `provider.temperature` → 未设置（不发送）。
- `ChatClient::prepare` 统一施加钉住覆盖：所有请求目的（对话/压缩/标题/
  验证/规划）生效；未配置时 wire 行为零变化。
- `config::merge` 支持分层配置合并该字段（逐字段合并，不动 headers 追加语义）。

## 用户侧

默认配置即兼容仅接受固定温度的模型，无需任何额外配置；
`temperature` 钉住仅在有明确调优诉求时使用。

## 测试清单（功能 → 测试）

- 压缩不发送温度：`compaction_and_model::small_model_is_used_for_compaction_summary_call`（新增 `reqs[0].temperature.is_none()` 断言）
- 验证不发送温度：`autopilot_verify::verify_snapshot_truncates_transcript_to_window`（同上）
- 标题不发送温度：`recovery` 标题用例（同上，`small_model` 路由断言旁）
- 解析优先级/回退/未设置：`config::tests::temperature_for_prefers_named_provider_entry`
  / `temperature_for_falls_back_to_legacy_provider_block` / `temperature_for_none_when_unset_everywhere`
- 分层合并：`config::tests::merge_applies_provider_temperature_from_layered_config`
- wire 钉住覆盖（捕获真实请求体）：`provider_temperature::pinned_temperature_overrides_per_purpose_default`
  / `pinned_temperature_applies_when_call_site_omits_it`
- 未配置透传（默认行为不变）：`provider_temperature::unpinned_provider_passes_call_site_value_through`
  / `unpinned_provider_omits_temperature_when_call_site_omits_it`

## 实测范围

`cargo test -p opencoder-session --test compaction_and_model`（8）+ `autopilot_verify`（10）+
`--test recovery title`（1）+ `--test responses auxiliary`（2）+ `compaction` 过滤（26）、
`cargo test -p opencoder-core --lib config`（143）、`cargo test -p opencoder-llm`（185，含新增 wire 测试）、
`tui model_menu/lifecycle`、`local session_cmd` 定向通过（前次改动引入的结构体字面量补字段）；
触及 crate `cargo clippy --all-targets` 零新增警告；`cargo check --workspace --all-targets` 通过。
构建产物级 E2E（`cargo build --bin opencoder` + 本地 mock SSE provider 捕获真实请求体）：
小 prompt 开场 + `--continue` 大 prompt 触发压缩，全部 5 个出站请求（主对话/压缩摘要/标题 ×2 轮）
均无 `temperature` 字段且压缩成功（`[context compacted]`）；钉住 provider（`"temperature": 1`）
的对照运行中全部请求携带 `temperature: 1.0`，确认可调优路径生效。
