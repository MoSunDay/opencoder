# fix(web/test): 配置端点测试隔离 — 阻止 host 配置污染

## 背景

`crates/web/tests/web_api_ops.rs` 中的 `get_config_returns_json` 与
`patch_config_merges_and_persists` 两个 integration 测试直接调用
`get_config` / `patch_config` handler，但未安装配置发现隔离
（`scoped_config_home`）。在 `HOME=/root` 的宿主机上：

- `get_config` 会读取真实 `~/.opencoder/config.json`（含宿主机密钥/值）。
- `patch_config` 通过 `Config::save(&state.workdir, …)` 写回**全局**
  配置文件路径（`config_home_dir()` 解析到 `~/.opencoder/`），而非测试
  临时目录。`patch_config_merges_and_persists` 以硬编码字面量
  `"claude-test-model"` 覆盖了真实配置的 `model` 字段，造成持久性损坏。

## 变更

仅修改 `crates/web/tests/web_api_ops.rs`（测试文件，无生产代码改动）：

1. **配置隔离**：为两个测试在 handler 调用前安装
   `let _iso = opencoder_core::scoped_config_home(state.workdir.clone());`。
   该 thread-local RAII 守卫使 `config_candidates` 全部解析到 override
   目录、`env_get` 对所有 env 名返回 `None`，确保
   `Config::load` / `Config::save` 只读写测试临时目录，永不触碰真实
   `~/.opencoder/config.json` 或宿主机 env 叠加层（`OPENCODER_MODEL` /
   `OPENAI_API_KEY` 等）。

2. **断言值派生**：`patch_config_merges_and_persists` 原硬编码
   `"claude-test-model"`，改为从隔离作用域内加载的配置派生：
   `let before = Config::load(&state.workdir).unwrap(); let new_model =
   format!("test-{}", before.model);`。测试保持确定性，同时消除与真实
   磁盘配置可能冲突的魔法字面量；断言 `assert_eq!(cfg.model, new_model)`
   验证的是磁盘持久化的可观测值，而非构造对象自证。

## 兼容性

- 无生产代码变更（`Config::save` / `save_target` / `scoped_config_home`
  实现均未触及）。
- `scoped_config_home` 为既有 API，生产代码从不调用它，行为不变。
- 测试仅在自身线程内临时覆盖配置发现，守卫 drop 后（即便 panic 经
  unwind）即恢复原状，无跨测试副作用。

## 既有损坏修复

早期未隔离运行已将真实 `~/.opencoder/config.json` 的 `model` 字段改写为
`claude-test-model`。本次将该字段恢复为 `glm-5.2/glm-5.2`（修复既有损坏；
本测试变更阻止未来再次污染）。

## 测试清单

- `cargo build --workspace` — 通过（0 warning）
- `cargo clippy --workspace --all-targets -- -D warnings` — 通过（0 warning）
- `cargo test -p opencoder-web --test web_api_ops` — 12 passed; 0 failed
- `cargo test --workspace` — 1830 passed; 0 failed; 1 ignored（预存豁免）

### 隔离验证

连跑 3 次 `patch_config_merges_and_persists` 后，真实
`~/.opencoder/config.json` 的 `model` 保持 `glm-5.2/glm-5.2` 未被改写，
反证隔离生效（修复前单次运行即将其改写为派生值
`test-<original_model>`）。
