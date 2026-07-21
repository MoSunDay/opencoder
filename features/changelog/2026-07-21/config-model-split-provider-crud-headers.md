# `/config` 与 `/model` 拆分 + Provider CRUD + 自定义 HTTP Headers

## 背景

此前 `/model` 是一个统一 5 字段弹窗（model / base_url / api_key / reasoning / threshold），将「生成参数」与「Provider 端点配置」混在一起。随着多 Provider（`providers` 命名表）、自定义 HTTP headers、能力开关等需求的增长，单弹窗已不堪重负。本轮将其拆为两个职责清晰的命令，并在 core 层补齐 `ProviderConfig` + `HttpHeader` 类型与端点解析。

## 变更

### Phase A — 自定义 HTTP Headers（core + llm）

- **`crates/core/src/config.rs`**：新增 `HttpHeader { name, value }`、`Endpoint { base_url, api_key, headers }`；`ProviderConfig` 增 `headers: Vec<HttpHeader>`；`resolve_endpoint() -> Result<Endpoint>` 返回 env 解析后的端点（header value 支持 `{VAR}` 环境变量间接）；`merge_into` 解析 headers 数组。`#[serde(default)]` 保证旧 config.json 向后兼容。
- **`crates/core/src/lib.rs`**：导出 `Endpoint`、`HttpHeader`。
- **`crates/llm/src/client.rs`**：`ChatClient` 增 `headers` 字段；`new` / `new_with_read_timeout` 签名改为 `&[(String, String)]`；纯函数 `build_header_map(key, custom)` 先加内置 header（Authorization / Content-Type / Accept），再按大小写不敏感名称覆盖自定义项，畸形条目静默跳过。
- **`crates/llm/src/lib.rs`**：导出 `build_header_map`。
- 全部 5 处生产调用点（worker.rs / app.rs×2 / api.rs / run.rs）已接线。

### Phase B — `/config` 瘦身 + `/model` Provider CRUD（tui）

**职责拆分**：
| 命令 | 拥有字段 |
|------|---------|
| `/config` | `reasoning_effort`, `interleaved_thinking`, `max_tokens`, `context_threshold`, `fps`, `capabilities.*`, `cache_salt`, `agent.*` |
| `/model` | `model_id`, `base_url`, `api_key`, `headers`（通过 `providers[name]` + `model = "{name}/{id}"`） |

**新建模块**（`crates/tui/src/model_menu/`）：
- **`patch.rs`**（106 行）：`ConfigPatch` / `ProviderPatch` 的 `to_json()` 产出 JSON merge-patch；`delete_provider_json()` / `switch_provider_json()` 辅助函数。api_key 未编辑则省略，清空则写 `null`（触发 `merge_json` 删除 key），env var 名包装为 `{VAR}`。
- **`headers.rs`**（226 行）：`HeadersEditor` — ↑/↓ 选 pair，←/→ 切 name↔value，`+` 新增，`-` 删除，字符编辑。7 个内联单测。
- **`config_form.rs`**（246 行）：`ConfigForm` + `ConfigField`（10 字段）+ `Reasoning` 枚举，`/config` 键处理。
- **`provider_form.rs`**（206 行）：`ProviderForm` + `ProviderField`（7 字段），`/model` 新增/编辑表单，含 headers 子模式路由。
- **`list.rs`**（144 行）：`ProviderList` + `ProviderEntry`，Enter=切换 / e=编辑 / n=新建 / d=删除（y/n 两步确认）。
- **`state.rs`**（69 行）：`ModelMenu { Config, List, Form }` + `ModelOutcome { Idle, Save(Value), Cancel, Quit }` + `handle_model_key` 分发（slot.take() 所有权模式）。
- **`view.rs`**（247 行）：`render_model_popup` 分发到三个变体。

**接线**：
- **`crates/tui/src/model_menu/mod.rs`**（32 行）：声明子模块 + re-export 新公共 API。
- **`crates/tui/src/app.rs`**：`ModelOutcome::Save` 现携带 `serde_json::Value`（直接 `Config::save`）；构造调用改为 `ModelMenu::List(ProviderList::new(...))` / `ModelMenu::Config(ConfigForm::new(...))`。
- **`crates/tui/src/keybind.rs`**：帮助文本更新为 `/config (settings), /model (providers)`。

### Phase C — 文档

- **`README.md`** / **`README.en.md`** §配置：补充 `providers` 命名表示例 + `model = "{provider}/{id}"` 前缀约定 + `headers` 字段。
- **`agents/tui/index.md`**：弹窗锚点段落重写，文档化 `/config`（生成参数）与 `/model`（Provider CRUD + headers）。
- **`agents/core/index.md`**：补充 `providers` map + `ProviderConfig` + `HttpHeader` + `resolve_endpoint`。
- **`features/index.md`**：能力摘要更新。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| build_header_map 6 场景 | `tests::*`（6 test） | `crates/llm/tests/headers.rs` |
| resolve_endpoint headers 解析 | `tests::*` | `crates/core/src/config.rs` |
| mask_key 短/长 key | `mask_hides_short_keys_entirely` | `model_menu/tests/common.rs` |
| Reasoning 循环 | `reasoning_cycle_is_circular` | `model_menu/tests/common.rs` |
| ConfigPatch 序列化 | `config_patch_serializes_all_fields` / `_omits_max_tokens_*` | `model_menu/tests/config_tests.rs` |
| ConfigForm 字段链 / reasoning / fps / max_tokens | `enter_chains_*` / `left_right_*` / `typing_digits_*`（7 test） | `model_menu/tests/config_tests.rs` |
| ProviderPatch env 包装 / 省略 / 清空 / headers | `provider_patch_*`（4 test） | `model_menu/tests/provider_tests.rs` |
| delete / switch patch | `delete_and_switch_patches` | `model_menu/tests/provider_tests.rs` |
| ProviderList 构建 / 切换 / 编辑 / 新建 / 删除 | `provider_list_*` / `list_*`（6 test） | `model_menu/tests/provider_tests.rs` |
| ProviderForm 保存 / api_key 编辑 | `provider_form_*`（2 test） | `model_menu/tests/provider_tests.rs` |
| HeadersEditor pair 导航 / 编辑 / 删除 | `tests::*`（7 test） | `model_menu/headers.rs` |
| Esc 取消各模式 | `esc_cancels_any_mode` | `model_menu/tests/provider_tests.rs` |

- 全量回归：`cargo test --workspace` → 769 passed / 0 failed / 0 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- 行数：所有 model_menu 新文件 ≤ 400 行（view.rs 247 最大）；mod.rs 32 行

## Gate

| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | 0 警告 | 0 警告 |
| 测试 | 763 passed | 769 passed |
| model_menu 文件 | mod.rs + state.rs + view.rs（单文件 ~800+ 行） | 8 文件拆分，最大 247 行 |

## Impact Surface

- **向后兼容**：`#[serde(default)]` 保证旧 config.json（无 `headers` 字段）正常加载；`/config` 去掉的 model/base_url/api_key 字段仍可通过 `/model` 或直接编辑 config.json 配置。
- **行为变更**：`/model` 从单表单变为列表 + CRUD 流程；`/config` 仅含生成参数。
- **新增能力**：自定义 HTTP headers（env 解析、大小写不敏感覆盖）；Provider 命名表 CRUD。
- **不受影响**：session 运行时（runner.rs / compaction.rs）、store 编码、web 路由、CLI 子命令。

## Notes

- api_key 编辑采用「掩码 + 重新输入」策略：未编辑时显示 `xx****xxxx`，开始输入即标记 `edited`，保存时仅写新值（不泄露掩码）。
- headers 的 value 支持 `{ENV_VAR}` 间接引用（与 api_key 同一约定），`resolve_endpoint` 运行时解析。
- Provider 删除通过 `providers: { "name": null }` merge-patch 实现（`merge_json` 已支持 Null 删 key）。

## Related Docs
- [model-command-and-reasoning-effort.md](../2026-07-05/model-command-and-reasoning-effort.md) — 原 `/model` 命令（已被拆分取代）
- [slash-popup-ux.md](../2026-07-11/slash-popup-ux.md) — 弹窗 UX（锚点逻辑不变）
- [browser-computer-use-proxy-integration.md](../2026-07-19/browser-computer-use-proxy-integration.md) — `/config` 能力开关
