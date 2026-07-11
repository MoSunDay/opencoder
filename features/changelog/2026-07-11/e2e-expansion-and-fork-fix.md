Commit: (working-tree, pre-initial-commit)

# e2e 补齐 5 项缺漏 + --fork 死 flag 修复

## 背景
e2e 脚本 `scripts/e2e-glm.sh` 只覆盖 4 项（E7/E1/E2/E6），压缩路径、subagent、`--fork`、bundle 导出/导入均无 e2e 验证。其中 `--fork` CLI flag 虽有声明和参数解析测试，但从未接入 `run_headless`——是死 flag。

## 变更

### A — `--fork` 死 flag 修复（`cli/src/run.rs`）
- 新增 `pub async fn fork_session(store: &dyn Store, parent_id: &str) -> Result<String>`：读 parent meta+messages → 新 id → `create_session` + `append_messages` → 打印 `[forked SID → NEW_SID]` → 返回新 id。
- `run_headless` 在 `pick_resume_id` 返回 id 后检查 `cli.fork`：为 true 则调 `fork_session` 获得新 id 再 `resume_session`，原 session 零修改。
- 新增 `cli/tests/fork_session.rs`（3 测试）：fork 复制消息且原 session 不变、fork 后 child 独立增长、fork 不存在 session 报错。

### B — e2e 新增 5 项（`scripts/e2e-glm.sh`）

| 测试 | 覆盖点 | 实现 |
|------|--------|------|
| **E3** | 压缩自动触发 | 独立 workdir `context_threshold:2000/tail_turns:1`，3 轮 calc.py 多函数开发，grep `[context compacted]` 排除 `nothing to compact` |
| **E3b** | 压缩后 `--continue` | 第 4 轮续写 test_calc.py，验证 session id 正常恢复 |
| **E4** | subagent (task tool) | 独立 workdir 含 hello.py，显式要求用 task 工具派遣 explore 子代理，grep `subagent [` |
| **E5** | `--fork` | 用 E1 的 SID 执行 `--session SID --fork`，验证新 session id ≠ 原始 id |
| **E8** | bundle 导出/导入 | `session export SID → .opencode` → `session import`，验证 roundtrip |

### 脚本重构
- 提取 `run()` 辅助函数统一 `( cd dirname && "$BIN" "$@" ) 2>&1 || true` 模式，减少重复。
- 新增 `$COMPACT`（低阈值 config）和 `$PROBE`（含 seed 文件）两个 workdir。

## 涉及文件
- `crates/cli/src/run.rs` — 新增 `fork_session` + 接入 `run_headless`
- `crates/cli/tests/fork_session.rs` — 3 个新测试（新建文件）
- `scripts/e2e-glm.sh` — 从 73 行扩展到 ~120 行，新增 E3/E3b/E4/E5/E8

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 271 passed, 1 failed (cancel_reset) | 276 passed, 0 failed |
| e2e 测试项 | 4 (E7/E1/E2/E6) | 9 (E7/E1/E2/E5/E3/E3b/E4/E6/E8) |

## 注意
- E3 压缩触发依赖模型生成量——`context_threshold:2000` + 3 轮多函数 prompt 确保累积 estimated tokens 超阈值。如模型输出极短可能不触发，属模型行为而非代码缺陷。
- E4 subagent 派遣依赖模型选择使用 task 工具——prompt 显式要求但不保证。失败时 check 输出标注 "model may not use task tool"。
- `cancel_reset` 此前失败（pre-existing `worker.rs` 未提交改动），现已通过。
