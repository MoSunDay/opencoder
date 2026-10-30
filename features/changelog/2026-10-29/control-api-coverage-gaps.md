# 控制面 API 覆盖补齐：发布状态、私有档案列表、能力详情与 Hello 门控

一句话：补齐控制面五处 API 覆盖缺口（过程内 e2e + 一个测试注入旋钮）：`GET /api/admin/release`（无平台配置与有平台配置两态）、`GET /api/harnesses/codex/profiles` 列表往返与修订递增、Ontology 能力详情 404/422 契约、`GET /api/resource-admission-provider` 默认读；节点通道首帧（Hello）等待超时改为按 `Lifecycle` 状态注入，默认 10s 行为不变。

## 变更清单

- `crates/control/src/release/mod.rs`：`Lifecycle` 新增 `hello_timeout_secs: OnceLock<u64>` 与 `hello_timeout()`（未设置即 `DEFAULT_HELLO_TIMEOUT_SECS = 10`，生产默认不变；按状态隔离，测试注入互不影响）。
- `crates/control/src/transport/socket.rs`：`serve()` 的首帧等待改读 `state.lifecycle.hello_timeout()`；新增 `#[doc(hidden)] set_hello_timeout_for_tests(&AppState, u64)` 测试注入点。
- `crates/control/src/transport/mod.rs`：导出注入函数。
- 新测试文件：`crates/control/tests/e2e/release_status.rs`、`crates/control/tests/e2e/codex_profiles.rs`、`crates/control/tests/node_keepalive/hello_timeout.rs`；扩写 `crates/control/tests/e2e/resource_admission.rs`、`crates/control/tests/ontology_catalog.rs`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 发布状态：无平台配置返回 `{"enabled":false}` | `release_status_without_platform_reports_disabled` | `crates/control/tests/e2e/release_status.rs` |
| 发布状态：合并实例 release、状态文件与 host 服务健康 | `release_status_with_platform_merges_state_and_host` | `crates/control/tests/e2e/release_status.rs` |
| 私有档案列表空起步、保存往返、修订递增 | `profiles_list_roundtrip_and_empty_start` | `crates/control/tests/e2e/codex_profiles.rs` |
| 能力详情返回列表同一文档；未知 UUID 404、非 UUID 422 | `capability_detail_returns_the_listed_document` | `crates/control/tests/ontology_catalog.rs` |
| 默认资源准入提供方读反映配置态 | `default_provider_read_reflects_configured_state` | `crates/control/tests/e2e/resource_admission.rs` |
| 静默连接按注入超时关闭；身份不匹配立即关闭且不注册 | `missing_hello_closes_the_socket_after_the_injected_timeout` | `crates/control/tests/node_keepalive/hello_timeout.rs` |

## 定向回归

- `cargo test -p opencoder-control --test e2e release_status / codex_profiles / resource_admission` → 7 通过。
- `cargo test -p opencoder-control --test ontology_catalog` → 9 通过。
- `cargo test -p opencoder-control --test node_keepalive` → 12 通过（连跑 6 次全绿）；`--test release_signals --test release_handoff --test node_registration --test admission` → 12 通过。
- `cargo clippy -p opencoder-control --all-targets` → 0 警告。
- 未运行全量测试。
