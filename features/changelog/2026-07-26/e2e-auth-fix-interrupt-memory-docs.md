Commit: (working-tree, pre-initial-commit)

# fix(e2e/docs): web serve auth bug + cancel/interrupt/title/crash e2e + memory doc repair

## 根因与背景

review 发现两类阻塞问题：

1. **`web_scenarios.py` auth bug（P0，现存 E11 从未通过）**：`opencode serve`
   无条件启用 bearer-token 鉴权（无 `--token` 时自动生成 ULID），但 e2e harness
   既不传 `--token` 也不发 `Authorization` 头，导致每个请求（含 `/api/health`）
   返回 401。E11「steer + queue 投递契约」因此永远无法到达 ready 状态。
2. **记忆文档失真（违反 repair-on-touch）**：4 类过期/缺失。
3. **运行时鲁棒性契约零 e2e**：cancel/interrupt、title 生成、crash 恢复。

## 变更

### A. 记忆文档修复（纯文档，低风险）

- **`agents.md`**：crate 数 7→8；模块索引补 `client` 条目；drain 路径
  `runner.rs::run_loop` → `runner/mod.rs::run_loop`（commit 83091f9 拆分后过期），
  标注 `DOOM_THRESHOLD` 现位于 `runner/event.rs`。
- **`agents/client/index.md`（新增）**：补全第 8 个 crate 的记忆文档——
  远端瘦客户端（`Remote` / `SseFrameDecoder` / `opencode client` 子命令），
  与 `agents/web/index.md`（server 侧）配对。
- **测试数 384→1059（4 处）**：`README.md`、`README.en.md`、`docs/perf.md`、
  `features/index.md`，对齐当次 `cargo test --workspace` 实跑值
  (1059 passed / 0 failed / 0 ignored)。

### B. web e2e auth 修复 + E15 cancel/interrupt（`scripts/e2e/web_scenarios.py`）

- **auth 修复**：serve 以固定 `--token e2e-web-token` 启动；`_request` /
  `_wait_health` 统一附加 `Authorization: Bearer <token>`。E11 恢复可达。
- **E15 cancel/interrupt**：admit 长 steer prompt → 等 3s → `POST /interrupt`
  → 断言 `{ok:true}` → 等 3s → re-admit 简单 prompt → 轮询验证第 2 条 user
  消息之后出现 assistant 回复（证明 cancel token 刷新后 drain 可再 spawn、
  session 不死锁）。跨进程运行时核心契约，MockChatClient integration 无法覆盖。

### C. CLI e2e 新增（`scripts/e2e/cli_scenarios.py`）

- **E16 title 生成（SOFT）**：E1 session 跑完后 `session show --json` 断言
  `meta.title` 非空且长度 < 60（生成标题远短于原始 prompt）。小模型不可达
  时 SOFT skip。
- **E17 crash-mid-write 恢复（SOFT）**：启动长 headless prompt → 8s 后
  `SIGKILL` → `--continue` 续跑 → 断言 exit 0 + history 可 `show --json` 加载。
  WAL 持久性 + resume 契约的跨进程验证。

### D. 轻微

- `crates/web/tests/client_e2e.rs` 头注释纠正分层标注（integration layer，
  mock LLM，非真 e2e）。
- `agents/session/index.md` 补 bash_guard 说明：工作流护栏（plan 模式防误写），
  非安全沙箱——Act 模式下模型拥有完全权限。

## 未覆盖（显式声明）

- **doom-loop guard (DOOM_THRESHOLD=3) e2e**：LOW 优先级。需模型连续重复同一
  tool call 3× 才触发，无 key 无法构造。integration 层已有覆盖
  (`crates/session/tests/tool_failure_guard.rs`)，e2e 留待模型配合场景。
- **A4（AGENTS.md / .opencode/golive.md 缺失）**：经核实为误报——代码中的
  `AGENTS.md` 引用是 opencode 的**可选用户指令自动加载功能**（`~/.opencoder/AGENTS.md`），
  非仓库必需文件；活跃指令引用小写 `agents.md`（已存在），无落差需修。

## 测试清单（rules/02-regression-gate）

- `cargo test --workspace` → **1059 passed; 0 failed; 0 ignored**（代码无变更，
  文档/e2e 脚本不影响 Rust 测试）。
- `python3 -m py_compile scripts/e2e/web_scenarios.py scripts/e2e/cli_scenarios.py
  scripts/e2e/lib.py scripts/e2e_glm.py` → 全部通过。
- serve auth smoke test（实跑真二进制）：`/api/health` 带正确 token → 200；
  带错误 token → 401；不带 token → 401；`POST /interrupt` 带正确 token →
  `{"ok":true}` 200。证实 auth 修复端到端生效。
- e2e 实跑（E11/E15/E16/E17）需 `ZHIPU_API_KEY`，当前环境无 key，待有 key 环境验证。
