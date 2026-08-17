Commit: (working-tree, post-dc89cef)

# 测试套件行数债务清偿：control_cmd / config_contract 拆分（纯移动）

Review 第三轮 go-live 报告披露的两处存量行数超标（rules/02 gate 4）本轮清偿。纯测试代码机械移动：**不新增、不删除、不改名、不改断言**，工作区测试总数 2915 与拆分前精确守恒。

## 拆分布局

| 文件 | 状态 | 行数 | 测试数 |
|------|------|------|--------|
| `crates/session/tests/control_cmd.rs` | 保留改写 | 632（原 1076） | 8 |
| `crates/session/tests/compound_cmd.rs` | 新建 | 387 | 5 |
| `crates/session/tests/plain_skill_prompt.rs` | 新建 | 224 | 2 |
| `crates/core/tests/config_contract.rs` | 保留改写 | 507（原 842） | 20 |
| `crates/core/tests/config_providers.rs` | 新建 | 383 | 13 |

- session 拆分边界：`control_cmd.rs` 留 idle 短路 / queue drain / ClearContext resume×3 / steered 不泄露 / sentinel / clear_context_compound（`/act_clear_context` 同主题且不依赖 HOME 隔离）；HOME 隔离段（HOME_MUTEX/HomeGuard/lock_home）随复合命令与 skill 测试迁入两个新文件，各自持副本（集成测试文件为独立进程，进程间 env 互不可见）。
- core 拆分边界：`config_contract.rs` 留 merge/env 覆盖/默认值/发现顺序/reasoning/interleaved/save roundtrip；providers map（prefix 解析、deep-merge、custom headers、api_key 回退）与全局配置文件语义（ensure_global×2、save_global）13 例迁入 `config_providers.rs`，ENV_LOCK/isolated_home/HomeGuard 各持副本。

## 测试清单（rules/01）

迁移测试（名称逐字不变，仅换文件）：

| 主题 | 测试名 | 文件 |
|------|--------|------|
| 复合控制命令 | idle_compound_plan_arg_switches_then_runs 等 5 例 | `session/tests/compound_cmd.rs` |
| 裸 skill prompt | queue_plain_skill_prompt_resolves、steer_plain_skill_prompt_resolves | `session/tests/plain_skill_prompt.rs` |
| providers/全局配置 | providers_map_resolves_endpoint_by_prefix 等 13 例 | `core/tests/config_providers.rs` |

守恒核验：session 15=8+5+2、core 33=20+13（拆分前后逐 target 实跑相等）；`#[test]` 函数名集合与 HEAD 逐字 diff 一致。

## 回归 gate（rules/02）

- `cargo test --workspace` → **2915 passed / 0 failed**（= 基线 2915，纯移动零增删）✓
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告 ✓
- `cargo build --workspace` → 干净 ✓；5 个文件 `rustfmt --check` 干净 ✓
- e2e：不适用——未触及任何 src/，session runner 语义未变

## Impact Surface

- 用户/功能：零影响（无 src/ 改动、无 Cargo.toml 改动）。
- 不影响：`agents/*` 记忆文档对 `tests/control_cmd.rs` 的既有锚点（idle 短路、queue_drains、ClearContext resume、steered）全部仍留在原文件，被迁移测试无文档引用，无需 repair-on-touch。
