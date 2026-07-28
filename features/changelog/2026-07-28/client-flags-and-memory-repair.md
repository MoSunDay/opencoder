Commit: (working-tree, pre-initial-commit)

# feat(cli): client --agent/--model/--interrupt + 记忆文档修正（A–F）

## 背景

健康度评估发现两个问题：

1. **client 降级使用**：`crates/client/src/remote.rs` 的 `Remote` 已 100% 打通
   server 端点（create_session/post_prompt 的 agent/model、switch、interrupt、health），
   但 `opencode client` 子命令从未把 flag 接到这些能力——`agent`/`model` 恒传 `None`，
   interrupt/switch 从不调用。远端 client 实际只能用裸 prompt，降级了已打通的 API。
2. **记忆文档滞后 6 处**：CLI 子命令清单缺 `Ts`/`Client` 且 `Serve` 应为 `Server`；
   虚构 flag `--agent`/`--small-model`；Web 鉴权误标为非目标（实际已实现 `auth.rs`）；
   `/api/sessions/:id/seq` 端点未文档化；Store 方法数 26（实际 27）；测试数 1076（实际 1214）。

## 变更

### client CLI flag 接入（功能性增强）

`Remote` 无需改动——API 早已打通。仅补 CLI plumbing：

- **`crates/cli/src/lib.rs`**：`Client` variant 新增 `agent: Option<String>`（设远端
  session agent，create + per-prompt override 均透传）与 `interrupt: bool`（中止远端
  drain 后退出）。`--model` 复用既有全局 flag，不新增本地字段（与 `run --model` 同语义，
  DRY 且避开 clap 全局/本地同名冲突风险）。
- **`crates/cli/src/client.rs`**：`client_run` 扩展为 8 参（加 `#[allow(clippy::too_many_arguments)]`，
  与 `Remote::post_prompt` 一致）。`agent`/`model` 透传进 `create_session` 与 `post_prompt`。
  interrupt 路径：要求 `--session <id>` 或 `--continue` 解析出 session（从不新建——取消一个
  还不存在的 session 无意义），调 `client.interrupt(session_id)` 后打印 `[interrupted remote session <id>]` 退出。
- **`src/main.rs`**：Client dispatch 透传 `agent`/`interrupt`；`require(&p)` 仅在非 interrupt
  时执行（interrupt 模式无需 prompt）。

### 记忆文档修正（A–F，纯文档）

| # | 修正 | 文件 |
|---|------|------|
| A | 子命令清单 `run/tui/serve/config/models/session` → `run/tui/ts/server/client/config/models/session`；`Serve`→`Server` | `agents/cli/index.md`、`agents.md` |
| B | 删虚构 flag `--small-model`/`--agent`/`--serve`，补实际 flag `--image`/`--prompt-file` | `agents/cli/index.md`、`agents.md` |
| C | Web 鉴权由「非目标」改为「已实现 `auth.rs`（bearer token 中间件）」 | `agents/web/index.md` |
| D | 文档化 `GET /api/sessions/:id/seq` 端点（`get_event_seq`） | `agents/web/index.md` |
| E | Store 方法 26→27；workspace 测试 1076→1214 | `agents/store/index.md`、`features/index.md` |
| F | `agents.md` tui 入口由纯文本改为 markdown 链接 | `agents.md` |

附带：`agents/cli/index.md` line 1 `Commit:` 标记对齐兄弟文件；e2e 场景清单 E1–E14 → E1–E17
（补 E15 cancel / E16 title / E17 崩溃恢复）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| client --agent/--model/--interrupt 解析 | `client_subcommand_parses_agent_model_interrupt` | `crates/cli/tests/cli_parse.rs` |
| client 既有 flag 解析回归（新增 agent/interrupt 字段） | `client_subcommand_parses` | `crates/cli/tests/cli_parse.rs` |

- interrupt 需 session 的运行时逻辑：client.rs 内联 `bail!`；server 侧 cancel/switch 端点契约
  已由 `crates/web/tests/web_contract.rs` 覆盖，CLI 层为薄透传，解析测试 + 编译期类型安全
  （main.rs 解构强制全字段透传）保证正确性。
- 全量回归：`cargo test --workspace` → **1214 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished clean

## Impact Surface

- `opencode client` 用户现可用 `--agent build`、`--model glm-5.2` 把远端 session 的
  agent/model 一并设置；`--interrupt` 可脚本化中止远端运行中的 drain。
- 不影响：本地 headless（`run`）/ TUI / drain 语义 / Store / ChatStream 边界；
  server 端无任何改动（API 未变）。纯增量 flag + 文档。
